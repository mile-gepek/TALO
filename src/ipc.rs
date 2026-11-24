use std::{
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use gpui_kit::{App, AppContext, Entity, EventEmitter, Task};
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Name, prelude::*};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::CliCommand;

pub static APP_NAME: &str = "talo";
pub const SOCKET_NAME: &str = concat!("talo-", env!("CARGO_PKG_VERSION"), ".sock");

pub enum IpcRole {
    Primary(LocalSocketListener),
    Secondary,
}

fn socket_name(socket_path: PathBuf) -> io::Result<Name<'static>> {
    socket_path.to_fs_name::<GenericFilePath>()
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

pub fn socket_path_in(base: &Path) -> io::Result<PathBuf> {
    let dir = base.join(APP_NAME);
    create_private_dir(&dir)?;
    Ok(dir.join(SOCKET_NAME))
}

pub fn default_socket_path() -> io::Result<PathBuf> {
    let base = dirs::runtime_dir() // Linux: $XDG_RUNTIME_DIR
        .or_else(dirs::cache_dir) // macOS etc.: short, per-user
        .unwrap_or_else(std::env::temp_dir);

    socket_path_in(&base)
}

fn command_to_line(command: CliCommand) -> String {
    serde_json::to_string(&command).unwrap()
}

fn line_to_command(buffer: String) -> serde_json::Result<CliCommand> {
    serde_json::from_str(&buffer)
}

pub fn claim_or_forward(socket_path: PathBuf, command: Option<CliCommand>) -> io::Result<IpcRole> {
    let name = socket_name(socket_path)?;
    match ListenerOptions::new().name(name.clone()).create_sync() {
        Ok(listener) => Ok(IpcRole::Primary(listener)),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            match LocalSocketStream::connect(name.clone()) {
                Ok(mut conn) => {
                    if let Some(command) = command {
                        let command_as_string = command_to_line(command);
                        conn.write_all(command_as_string.as_bytes())?;
                    }
                    Ok(IpcRole::Secondary)
                }
                Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => {
                    let listener = ListenerOptions::new()
                        .name(name)
                        .try_overwrite(true)
                        .create_sync()?;
                    Ok(IpcRole::Primary(listener))
                }
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

pub fn spawn_socket_watcher(
    listener: LocalSocketListener,
    initial_command: Option<CliCommand>,
) -> Option<UnboundedReceiver<CliCommand>> {
    let (ipc_command_tx, ipc_command_rx) = tokio::sync::mpsc::unbounded_channel();

    if let Some(initial_command) = initial_command {
        _ = ipc_command_tx.send(initial_command);
    }

    std::thread::spawn(move || {
        let mut buffer = String::with_capacity(128);

        for mut conn in listener
            .incoming()
            .filter_map(|conn| {
                conn.map_err(|err| tracing::error!("Incoming connection failed: {err}"))
                    .ok()
            })
            .map(BufReader::new)
        {
            if let Err(err) = conn.read_line(&mut buffer) {
                tracing::debug!("Error reading line from socket: {err}");
                buffer.clear();
                continue;
            };
            drop(conn);
            match line_to_command(buffer.clone()) {
                Ok(command) => {
                    if ipc_command_tx.send(command).is_err() {
                        return;
                    };
                }
                Err(err) => {
                    tracing::error!("Got error deserializing IPC data: {err}");
                }
            };
            buffer.clear();
        }
    });

    Some(ipc_command_rx)
}

pub struct IpcService {
    _task: Task<()>,
}

impl EventEmitter<CliCommand> for IpcService {}

impl IpcService {
    pub fn start_with_listener(
        listener: LocalSocketListener,
        initial_command: Option<CliCommand>,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let mut ipc_command_rx = spawn_socket_watcher(listener, initial_command)?;

        let ipc_service = cx.new(|cx| {
            let task = cx.spawn(async move |ipc_service, cx| {
                while let Some(ipc_command) = ipc_command_rx.recv().await {
                    tracing::debug!("Got IPC command: {ipc_command:?}");

                    if ipc_service
                        .update(cx, |_, cx| {
                            cx.emit(ipc_command);
                        })
                        .is_err()
                    {
                        tracing::warn!("IPC service dropped while task was running");
                        break;
                    }
                }
            });

            Self { _task: task }
        });
        Some(ipc_service)
    }
}
