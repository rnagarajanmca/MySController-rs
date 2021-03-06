extern crate crossbeam_channel as channel;
#[macro_use]
extern crate diesel_migrations;
extern crate num_cpus;

extern crate actix;
extern crate actix_web;
extern crate diesel;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate ini;
extern crate myscontroller_rs;

use actix::*;
use actix_web::{http::Method, middleware, middleware::cors::Cors, server, App};
use diesel::prelude::SqliteConnection;
use diesel::r2d2::{ConnectionManager, Pool};

use ini::Ini;
use myscontroller_rs::api::index::AppState;
use myscontroller_rs::api::{firmware, index, node, sensor};
use myscontroller_rs::core::{connection, server as mys_controller};
use myscontroller_rs::model::db;
use myscontroller_rs::wot;
use std::fs::create_dir_all;
use std::path::Path;
use std::thread;

fn main() {
    embed_migrations!("migrations");

    let sys = actix::System::new("webapp");

    let conf = match Ini::load_from_file("/etc/myscontroller-rs/conf.ini") {
        Ok(_conf) => _conf,
        Err(_) => Ini::load_from_file("conf.ini").unwrap(),
    };

    ::std::env::set_var("RUST_LOG", log_level(&conf));
    ::std::env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();

    let database_url = server_configs(&conf);
    let manager = ConnectionManager::<SqliteConnection>::new(database_url);
    let conn = Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");
    let conn_clone = conn.clone();
    let database_addr = SyncArbiter::start(num_cpus::get() * 4, move || db::ConnDsl(conn.clone()));

    let (controller_in_sender, controller_in_receiver) = channel::unbounded();
    let (out_set_sender, out_set_receiver) = channel::unbounded();
    let (in_set_sender, in_set_receiver) = channel::unbounded();
    let reset_signal_sender = controller_in_sender.clone();
    server::new(move || {
        App::with_state(AppState {
            db: database_addr.clone(),
            reset_sender: reset_signal_sender.clone(),
        }).middleware(middleware::Logger::default())
            .configure(|app| {
                Cors::for_app(app)
                    .allowed_methods(vec!["GET", "PUT", "POST", "DELETE"])
                    .resource("/", |r| {
                        r.method(Method::GET).h(index::home);
                    })
                    .resource("/nodes", |r| {
                        r.method(Method::GET).h(node::list);
                        r.method(Method::POST).with2(node::create);
                        r.method(Method::PUT).with2(node::update);
                        r.method(Method::DELETE).with2(node::delete);
                    })
                    .resource("/nodes/{node_id}", |r| {
                        r.method(Method::GET).h(node::get_node);
                    })
                    .resource("/nodes/{node_id}/reboot", |r| {
                        r.method(Method::POST).h(node::reboot_node);
                    })
                    .resource("/sensors", |r| {
                        r.method(Method::GET).h(sensor::list);
                        r.method(Method::DELETE).with2(node::delete);
                    })
                    .resource("/sensors/{node_id}/{child_sensor_id}", |r| {
                        r.method(Method::GET).h(sensor::get_sensor);
                    })
                    .resource("/firmwares", |r| {
                        r.method(Method::GET).h(firmware::list);
                    })
                    .resource("/firmwares/{firmware_type}/{firmware_version}", |r| {
                        r.method(Method::POST).with2(firmware::create);
                        r.method(Method::PUT).with2(firmware::update);
                        r.method(Method::DELETE).with(firmware::delete);
                    })
                    .resource("/firmwares/upload", |r| {
                        r.method(Method::GET).with(firmware::upload_form);
                    })
                    .register()
            })
    }).bind("0.0.0.0:8000")
        .unwrap()
        .shutdown_timeout(3)
        .start();

    match conn_clone.get() {
        Ok(conn) => embedded_migrations::run_with_output(&conn, &mut std::io::stdout()).unwrap(),
        Err(e) => error!("Error while running migration {:?}", e),
    };

    info!("Starting proxy server");

    let conn_pool_clone = conn_clone.clone();

    thread::spawn(move || {
        mys_controller::start(
            get_mys_gateway(&conf),
            get_mys_controller(&conf),
            conn_clone,
            controller_in_sender,
            controller_in_receiver,
            in_set_sender,
            out_set_receiver,
        );
    });

    wot::start_server(conn_pool_clone, out_set_sender, in_set_receiver);
    sys.run();
}

pub fn server_configs(config: &Ini) -> String {
    let server_conf = config
        .section(Some("Server".to_owned()))
        .expect("Server configurations missing");
    let database_url = server_conf.get("database_url").expect(
        "database_url is not specified. Ex:database_url=/var/lib/myscontroller-rs/sqlite.db",
    );
    let database_path = Path::new(database_url);
    create_dir_all(database_path.parent().unwrap()).unwrap();
    database_url.to_owned()
}

pub fn log_level(config: &Ini) -> String {
    config
        .get_from(Some("Server"), "log_level")
        .unwrap_or("myscontroller_rs=info,actix_web=info")
        .to_owned()
}

fn get_mys_controller<'s>(config: &'s Ini) -> connection::StreamInfo {
    let controller_conf = config.section(Some("Controller".to_owned())).unwrap();
    let controller_type = controller_conf.get("type").expect("Controller port is not specified. Ex:\n\
     [Controller]\n type=SERIAL\n port=/dev/tty1\n or \n\n[Controller]\n type=SERIAL\n port=port=0.0.0.0:5003");
    let controller_type = match connection::ConnectionType::from_str(controller_type.as_str(), true)
    {
        Some(value) => value,
        None => panic!("Possible values for type is TCP or SERIAL"),
    };
    let controller_port = controller_conf.get("port").expect("Controller port is not specified. Ex:\n\
     [Controller]\n type=SERIAL\n port=/dev/tty1\n or \n\n[Controller]\n type=SERIAL\n port=port=0.0.0.0:5003");
    connection::StreamInfo {
        port: controller_port.to_owned(),
        connection_type: controller_type,
    }
}

fn get_mys_gateway<'s>(config: &'s Ini) -> connection::StreamInfo {
    let gateway_conf = config.section(Some("Gateway".to_owned())).unwrap();
    let gateway_type = gateway_conf.get("type").expect("Gateway port is not specified. Ex:\n\
     [Gateway]\n type=SERIAL\n port=/dev/tty1\n or \n\n[Gateway]\n type=SERIAL\n port=port=10.137.120.250:5003");
    let gateway_type = match connection::ConnectionType::from_str(gateway_type.as_str(), false) {
        Some(value) => value,
        None => panic!("Possible values for type is TCP or SERIAL"),
    };
    let gateway_port = gateway_conf.get("port").expect("Gateway port is not specified. Ex:\n\
     [Gateway]\n type=SERIAL\n port=/dev/tty1\n or \n\n[Gateway]\n type=SERIAL\n port=port=10.137.120.250:5003");
    connection::StreamInfo {
        port: gateway_port.to_owned(),
        connection_type: gateway_type,
    }
}
