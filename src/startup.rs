//! Startup module.

use cc_utils::prelude::MResult;
use salvo::prelude::*;

use salvo::server::ServerHandle;
use std::future::Future;
use std::pin::Pin;
use std::process::Command;
use salvo::conn::rustls::{Keycert, RustlsConfig};

#[cfg(feature = "http3")]
use salvo::http::header::ALT_SVC;
#[cfg(feature = "http3")]
use salvo::http::HeaderValue;

#[cfg(feature = "oapi")]
use salvo::oapi::security::Http;
#[cfg(feature = "oapi")]
use salvo::oapi::SecurityScheme;

use crate::generic_setup::{GenericSetup, GenericValues, StartupVariant, GenericServerState};

#[cfg(feature = "http3")]
#[handler]
/// HTTP2-to-HTTP3 switching header.
/// 
/// Usage is `router.hoop(h3_header)`.
pub async fn h3_header(depot: &mut Depot, res: &mut Response) {
  let server_port = match depot.obtain::<GenericValues>() {
    Ok(app_config) => app_config.server_port.clone(),
    Err(_) => 443.to_string(),
  };

  res.headers_mut().insert(
    ALT_SVC,
    HeaderValue::from_str(&format!(r##"h3=":{}"; ma=2592000"##, server_port)).unwrap()
  ).unwrap();
}

pub fn get_root_router_autoinject<T: GenericSetup + Send + Sync + Clone + 'static>(app_state: &GenericServerState, app_config: T) -> Router {
  #[allow(unused_mut)] let mut router = Router::new().hoop(affix_state::inject(app_state.clone()).inject(app_config));

  #[cfg(all(feature = "http3", feature = "acme"))]
  if app_state.startup_variant == StartupVariant::QuinnAcme {
    router = router.hoop(h3_header);
  }
  
  #[cfg(feature = "http3")]
  if
    app_state.startup_variant == StartupVariant::Quinn ||
    app_state.startup_variant == StartupVariant::QuinnOnly
  {
    router = router.hoop(h3_header);
  }

  router
}

/// Returns preconfigured root router to use.
/// 
/// Usually it installs application config and state in `affix_state` and installs `h3_header` for switching protocol to QUIC, if used.
pub fn get_root_router(app_state: &GenericServerState) -> Router {
  #[allow(unused_mut)] let mut router = Router::new();
  
  #[cfg(all(feature = "http3", feature = "acme"))]
  if app_state.startup_variant == StartupVariant::QuinnAcme {
    router = router.hoop(h3_header);
  }
  
  #[cfg(feature = "http3")]
  if
    app_state.startup_variant == StartupVariant::Quinn ||
    app_state.startup_variant == StartupVariant::QuinnOnly
  {
    router = router.hoop(h3_header);
  }

  router
}

/// Starts the server according to the startup variant provided with the custom shutdown.
pub async fn start_with_custom_shutdown<F, Fut>(
  app_state: GenericServerState,
  app_config: &impl GenericSetup,
  #[allow(unused_mut)] mut router: Router,
  custom_shutdowns: &[F],
) -> MResult<(Pin<Box<dyn Future<Output = ()> + Send>>, ServerHandle)>
where
  F: FnOnce(ServerHandle) -> Fut,
  Fut: Future<Output = ()> + Send + 'static,
{
  tracing::info!("Server is starting...");
  
  let app_config = app_config.generic_values();
  
  if let Some(bin) = app_config.auto_migrate_bin.as_ref() {
    Command::new(bin).spawn()?;
  }

  #[cfg(feature = "oapi")]
  if app_config.allow_oapi_access.is_some_and(|v| v) {
    let doc = OpenApi::new(app_config.oapi_name.as_ref().unwrap(), app_config.oapi_ver.as_ref().unwrap())
      .add_security_scheme(
        "bearer",
        SecurityScheme::Http(Http::new(salvo::oapi::security::HttpAuthScheme::Bearer).bearer_format("JSON"))
      )
      .merge_router(&router);
    
    let oapi_endpoint = if let Some(ftype) = app_config.oapi_frontend_type.as_ref() {
      match ftype.as_str() {
        "Scalar" => Some(Scalar::new(format!("{}/openapi.json", app_config.oapi_api_addr.as_ref().unwrap()))
          .title(format!("{} - API @ Scalar", app_config.oapi_name.as_ref().unwrap_or(&app_config.app_name)))
          .description(format!("{} - API", app_config.oapi_name.as_ref().unwrap_or(&app_config.app_name)))
          .into_router(app_config.oapi_api_addr.as_ref().unwrap())),
        "SwaggerUI" => Some(SwaggerUi::new(format!("{}/openapi.json", app_config.oapi_api_addr.as_ref().unwrap()))
          .title(format!("{} - API @ SwaggerUI", app_config.oapi_name.as_ref().unwrap_or(&app_config.app_name)))
          .description(format!("{} - API", app_config.oapi_name.as_ref().unwrap_or(&app_config.app_name)))
          .into_router(app_config.oapi_api_addr.as_ref().unwrap())),
        _ => None,
      }
    } else { None };
    
    let old_router = router;
    router = Router::new();
    
    router = router.push(doc.into_router(format!("{}/openapi.json", app_config.oapi_api_addr.as_ref().unwrap())));
    if let Some(oapi) = oapi_endpoint { router = router.push(oapi); }
    
    router = router.push(old_router);
    tracing::info!("API is available on {}", app_config.oapi_api_addr.as_ref().unwrap());
  }
  
  let handle;
  
  let mk_service = |router: Router, #[allow(unused)] app_config: &GenericValues| {
    #[allow(unused_mut)] let mut service = Service::new(router);
    
    #[cfg(feature = "cors")]
    if let Some(domain) = &app_config.allow_cors_domain {
      let cors = salvo::cors::Cors::new()
        .allow_origin(domain)
        .allow_credentials(if domain.as_str() == "*" { false } else { true })
        .allow_headers(vec![
          "Authorization",
          "Accept",
          "Access-Control-Allow-Headers",
          "Content-Type",
          "Origin",
          "X-Requested-With",
        ])
        .allow_methods(vec![
          salvo::http::Method::GET,
          salvo::http::Method::POST,
          salvo::http::Method::PUT,
          salvo::http::Method::PATCH,
          salvo::http::Method::DELETE,
          salvo::http::Method::OPTIONS,
        ])
        .into_handler();
      
      service = service.hoop(cors);
    }
    
    service
  };
  
  let server = match app_state.startup_variant {
    StartupVariant::HttpLocalhost => {
      let acceptor = TcpListener::new(format!("127.0.0.1:{}", app_config.server_port)).bind().await;
      let server = Server::new(acceptor);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config))) as Pin<Box<dyn Future<Output = ()> + Send>>
    },
    StartupVariant::UnsafeHttp => {
      let acceptor = TcpListener::new(format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port)).bind().await;
      let server = Server::new(acceptor);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config)))
    },
    #[cfg(feature = "acme")]
    StartupVariant::HttpsAcme => {
      let acme_listener = TcpListener::new(format!("0.0.0.0:{}", app_config.acme_challenge_port.as_ref().unwrap()))
        .acme()
        .cache_path("tmp/letsencrypt")
        .add_domain(app_config.acme_domain.as_ref().unwrap())
        .http01_challenge(&mut router);
      let acceptor = acme_listener.join(TcpListener::new(format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port))).bind().await;
      let server = Server::new(acceptor);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config)))
    },
    StartupVariant::HttpsOnly => {
      let rustls_config = RustlsConfig::new(
        Keycert::new().cert_from_path(app_config.ssl_crt_path.as_ref().unwrap())?.key_from_path(app_config.ssl_key_path.as_ref().unwrap())?
      );
      let listener = TcpListener::new(
        format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port)).rustls(rustls_config.clone()
      ).bind().await;

      let server = Server::new(listener);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config)))
    },
    #[cfg(all(feature = "http3", feature = "acme"))]
    StartupVariant::QuinnAcme => {
      let acme_listener = TcpListener::new(format!("0.0.0.0:{}", app_config.acme_challenge_port.as_ref().unwrap()))
        .acme()
        .cache_path("tmp/letsencrypt")
        .add_domain(app_config.acme_domain.as_ref().unwrap())
        .http01_challenge(&mut router)
        .quinn(format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port));
      let acceptor = acme_listener.join(TcpListener::new(format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port))).bind().await;
      let server = Server::new(acceptor);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config)))
    },
    #[cfg(feature = "http3")]
    StartupVariant::Quinn => {
      let rustls_config = RustlsConfig::new(
        Keycert::new().cert_from_path(app_config.ssl_crt_path.as_ref().unwrap())?.key_from_path(app_config.ssl_key_path.as_ref().unwrap())?
      );
      let listener = TcpListener::new(
        format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port)).rustls(rustls_config.clone()
      );

      let quinn_config = RustlsConfig::new(
        Keycert::new().cert_from_path(app_config.ssl_crt_path.as_ref().unwrap())?.key_from_path(app_config.ssl_key_path.as_ref().unwrap())?
      ).alpn_protocols(vec!["h3".as_bytes().to_owned()]).build_quinn_config()?;
      let acceptor = QuinnListener::new(
        quinn_config,
        format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port)
      ).join(listener).bind().await;

      let server = Server::new(acceptor);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config)))
    },
    #[cfg(feature = "http3")]
    StartupVariant::QuinnOnly => {
      let quinn_config = RustlsConfig::new(
        Keycert::new().cert_from_path(app_config.ssl_crt_path.as_ref().unwrap())?.key_from_path(app_config.ssl_key_path.as_ref().unwrap())?
      ).alpn_protocols(vec!["h3".as_bytes().to_owned()]).build_quinn_config()?;
      let acceptor = QuinnListener::new(
        quinn_config,
        format!("{}:{}", app_config.server_host.as_ref().unwrap(), app_config.server_port)
      ).bind().await;

      let server = Server::new(acceptor);
      handle = server.handle();
      
      for fut in custom_shutdowns {
        let handle = server.handle();
        tokio::spawn(fut(handle));
      }
      
      Box::pin(server.serve(mk_service(router, &app_config)))
    },
  };
  
  Ok((server, handle))
}

/// Starts the server according to the startup variant provided.
pub async fn start(
  app_state: GenericServerState,
  app_config: &impl GenericSetup,
  router: Router,
) -> MResult<(Pin<Box<dyn Future<Output = ()> + Send>>, ServerHandle)> {
  start_with_custom_shutdown::<fn(ServerHandle) -> std::future::Ready<()>, _>(app_state, app_config, router, &[default_shutdown_signal]).await
}

pub async fn default_shutdown_signal(handle: ServerHandle) {
  tokio::signal::ctrl_c().await.unwrap();
  tracing::info!("Shutdown with Ctrl+C requested.");
  handle.stop_graceful(None);
}
