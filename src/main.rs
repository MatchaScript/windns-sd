use astro_dnssd::DNSServiceBuilder;
use config::{Config, ConfigError, File};
use serde::Deserialize;
use std::net::TcpListener;
use std::{env, ffi::OsString, io, path::Path, sync::mpsc, time::Duration};
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher, Result,
};

#[derive(Debug, Deserialize)]
struct ServiceConfig {
    name: String,
    #[serde(rename = "type")]
    service_type: String,
    port: u16,
    text: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct Settings {
    services: std::collections::HashMap<String, ServiceConfig>,
}

impl Settings {
    pub fn from_file(config_path: &Path) -> std::result::Result<Self, ConfigError> {
        let config = Config::builder()
            .add_source(File::from(config_path))
            .build()?;
        config.try_deserialize()
    }
}

define_windows_service!(ffi_service_main, windns_sd_service_main);
const SERVICE_NAME: &str = "windns-sd";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

fn available_port() -> io::Result<u16> {
    match TcpListener::bind("localhost:0") {
        Ok(listener) => {
            let port = listener.local_addr()?.port();
            Ok(port)
        }
        Err(e) => Err(e),
    }
}

fn windns_sd_service_main(_arguments: Vec<OsString>) {
    if let Err(_e) = run_service() {
        // Handle the error, by logging or something.
    }
}

fn run_service() -> Result<()> {
    // Create a channel to be able to poll a stop event from the service worker loop.
    let (service_control_tx, service_control_rx) = mpsc::channel();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            // Notifies a service to report its current status information to the service
            // control manager. Always return NoError even if not implemented.
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            // Handle stop
            ServiceControl::Stop => {
                service_control_tx.send(control_event).unwrap();
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
    // Tell the system that service is running
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    // Start the service worker loop
    // read config file from $ProgramData/windns-sd/config.toml
    // use env::var("ProgramData") to get the path
    let config_path = Path::new(&env::var("ProgramData").unwrap())
        .join("windns-sd")
        .join("config.toml");
    let config = crate::Settings::from_file(&config_path).unwrap();
    for (_service_name, service_config) in &config.services {
        let service_type = &service_config.service_type;
        let service_hostname = &service_config.name;
        let port = if service_config.port == 0 {
            available_port().unwrap()
        } else {
            service_config.port
        };
        let properties = service_config.text.clone().unwrap_or_default();
        let mut service = DNSServiceBuilder::new(&service_type, port)
            .with_name(service_hostname)
            .with_txt_record(properties)
            .register();
        //create a new thread for each service
        std::thread::spawn(move || match service {
            Ok(mut service) => {
                std::thread::park();
            }
            Err(e) => {
                println!("Error registering service: {:?}", e);
            }
        });
    }
    loop {
        // Poll service control events from the channel.
        match service_control_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(control_event) => match control_event {
                // ServiceControl::Stop event is received, the loop exits.
                ServiceControl::Stop => {
                    status_handle.set_service_status(ServiceStatus {
                        service_type: SERVICE_TYPE,
                        current_state: ServiceState::StopPending,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::Win32(0),
                        checkpoint: 0,
                        wait_hint: Duration::default(),
                        process_id: None,
                    })?;
                    break;
                }
                _ => (),
            },
            Err(e) => println!("Error receiving service control event: {:?}", e),
        }
    }
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    Ok(())
}

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main);
    Ok(())
}
