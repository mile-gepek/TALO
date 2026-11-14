use std::{
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use gpui_kit::AsyncApp;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Name, prelude::*};

use crate::{
    CliCommand,
    breaks::{pause, resume, run_break_forced},
};

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

fn line_to_command(buffer: String) -> CliCommand {
    serde_json::from_str(&buffer).unwrap()
}

fn claim_or_forward(socket_path: PathBuf, command: Option<CliCommand>) -> io::Result<IpcRole> {
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

async fn handle_command(cx: &mut AsyncApp, command: CliCommand) {
    match command {
        CliCommand::Pause => _ = pause(cx),
        CliCommand::Resume => _ = resume(cx),
        CliCommand::RunBreak { break_type } => {
            run_break_forced(cx, break_type);
        }
        CliCommand::Quit => cx.update(|cx| cx.quit()),
        CliCommand::Settings => {
            todo!("Open settings screen");
        }
    }
}

pub async fn spawn_socket_watcher(cx: &mut AsyncApp, command: Option<CliCommand>) {
    let (ipc_command_tx, mut ipc_command_rx) = tokio::sync::mpsc::unbounded_channel();

    let listener = match claim_or_forward(default_socket_path().unwrap(), command) {
        Ok(IpcRole::Primary(listener)) => {
            if let Some(command) = command {
                handle_command(cx, command).await;
            }
            listener
        }
        Ok(IpcRole::Secondary) => {
            cx.update(|cx| cx.quit());
            return;
        }
        Err(err) => {
            tracing::error!("Error settings up socket: {err}");
            cx.update(|cx| cx.quit());
            return;
        }
    };

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
            conn.read_line(&mut buffer).unwrap();
            drop(conn);
            let command = line_to_command(buffer.clone());
            if ipc_command_tx.send(command).is_err() {
                return;
            };
            buffer.clear();
        }
    });

    while let Some(ipc_command) = ipc_command_rx.recv().await {
        tracing::debug!("Got IPC command: {ipc_command:?}");
        handle_command(cx, ipc_command).await;
    }
}
