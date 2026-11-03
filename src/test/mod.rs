use std::path::PathBuf;

#[cfg(test)]
mod logging_env;

#[cfg(feature = "client")]
pub use crate::client::test::get_simulator_devices;

#[cfg(feature = "server")]
pub use crate::server::test::{ConformUTestBuilder, run_conformu_tests};

pub(crate) fn resolve_path(path_hint: &'static str, exe_name: &'static str) -> PathBuf {
    use std::env;

    env::split_paths(&env::var_os("PATH").unwrap_or_default())
    .chain(std::iter::once(path_hint.into()))
    .map(|path| path.join(exe_name))
    .find(|path| path.exists())
    .unwrap_or_else(|| panic!("{exe_name} not found in either PATH or the standard installation directory {path_hint}"))
}

macro_rules! cmd {
    ($windows_path_hint:literal, $name:literal) => {
        tokio::process::Command::new(if cfg!(windows) {
            use std::sync::LazyLock;
            // On Windows, ASCOM binaries have well-known path that we can look up if executable is not on the global PATH.
            static RESOLVED_PATH: LazyLock<std::path::PathBuf> = LazyLock::new(|| {
                $crate::test::resolve_path($windows_path_hint, concat!($name, ".exe"))
            });
            &RESOLVED_PATH
        } else {
            // On other systems, just rely on the user adding binaries to the global PATH.
            std::path::Path::new($name)
        })
        .kill_on_drop(true)
        .stdin(Stdio::null())
    };
}

#[cfg(test)]
pub(crate) async fn run_proxy_tests<T: ?Sized + crate::api::RetrieavableDevice>() -> eyre::Result<()>
{
    use crate::Server;
    use crate::api::CargoServerInfo;
    use crate::client::test::get_simulator_devices;
    use crate::server::test::run_conformu_tests;
    use std::net::Ipv4Addr;

    let proxy = Server {
        devices: get_simulator_devices().await?.clone(),
        listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
        ..Server::new(CargoServerInfo!())
    };

    let proxy = proxy.bind().await?;

    // Get the IP and the random port assigned by the OS.
    let server_url = format!("http://{}/", proxy.listen_addr());

    tokio::select! {
        proxy_result = proxy.start() => match proxy_result? {},
        tests_result = run_conformu_tests::<T>(server_url, 0) => tests_result,
    }
}

/// An in-process Alpaca server and the devices it serves, as seen by an Alpaca client.
///
/// The server is shut down when this is dropped.
#[cfg(all(test, feature = "client", feature = "server"))]
#[derive(Debug)]
pub(crate) struct TestServer {
    /// The served devices, as seen through an Alpaca [`Client`](crate::Client).
    pub(crate) devices: crate::api::Devices,
    server: tokio::task::JoinHandle<eyre::Result<std::convert::Infallible>>,
}

#[cfg(all(test, feature = "client", feature = "server"))]
impl Drop for TestServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// Serves `devices` from an in-process Alpaca server on a random local port and
/// returns them as seen through an Alpaca client, so that a test can drive a device
/// implementation over the real wire protocol.
#[cfg(all(test, feature = "client", feature = "server"))]
pub(crate) async fn serve_devices(devices: crate::api::Devices) -> eyre::Result<TestServer> {
    use crate::api::CargoServerInfo;
    use crate::{Client, Server};
    use std::net::Ipv4Addr;

    let server = Server {
        devices,
        listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
        // Every test gets its own server, and they can't all share the discovery port.
        discovery_port: None,
        ..Server::new(CargoServerInfo!())
    }
    .bind()
    .await?;

    let client = Client::new_from_addr(server.listen_addr());
    let server = tokio::spawn(server.start());

    Ok(TestServer {
        devices: client.get_devices().await?.collect(),
        server,
    })
}
