# MyRController
Proxy controller for MySensors. It is to perform OTA firmware updates, and proxy all other requests to the actual controllers like homeassist. Mainly to add OTA support for homeassist controller, but can work with any other controllers.

This server acts as a proxy between Gateway and the Controller. Both might be either connected through a serial port or a TCP connection.

Before running the server, set the correct connection type and connection port for Gateway and Controller in conf.ini file.

To run the proxy server:
```
cargo run
```

Note: If you are using TCP for controller - the port value will be used to create TCP server listening on the specified port. (So it shoud be the address of the machine running MyRController)

## TODO

* Gracefully handle connection at both side, i.e never panic and wait for both connections
* Load firmwares from specific location and manage it's type and version
* Manage nodes and the firmwares installed
* Add an endpoint to send the firmware to particular node
* Manage requested firmware for nodes