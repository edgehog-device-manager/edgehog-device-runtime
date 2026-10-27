// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::{
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};

use astarte_device_sdk::Client;
use eyre::Context;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
};
use tracing::{error, info, instrument, trace, warn};

use crate::file_transfer::{
    file_system::WriteHandle,
    interface::{
        hooks::{HookPhase, HookResponse},
        status::FileTransferId,
    },
    request::{CommandHook, download::Download, upload::Upload},
};

#[derive(Debug)]
pub(crate) struct HookDef {
    command: Command,
    timeout: Duration,
}

impl HookDef {
    const KILL_TIMEOUT: Duration = Duration::from_secs(8);

    fn new(hook: &CommandHook<'_>) -> Self {
        let mut command = hook.command();
        let timeout = hook.timeout();

        command.kill_on_drop(true);

        #[cfg(windows)]
        {
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP);
        }

        Self { command, timeout }
    }

    fn set_env(mut self, id: &str, path: &Path, target_name: &str, target_value: &Path) -> Self {
        self.command.env("FT_REQUEST_ID", id);
        self.command.env("FT_REQUEST_PATH", path);
        self.command.env("FT_REQUEST_TARGET_NAME", target_name);
        self.command
            .env("FT_REQUEST_TARGET_VALUE", target_value.as_os_str());

        self
    }

    #[instrument(skip_all)]
    async fn run(&mut self) -> io::Result<(ExitStatus, Child)> {
        let mut child = self.command.spawn()?;
        let wait_exit = tokio::time::timeout(self.timeout, child.wait()).await;
        if let Ok(res) = wait_exit {
            return res
                .inspect(|status| trace!(?status, "process exited"))
                .map(|status| (status, child));
        };

        warn!("timeout reached terminating process");

        self.terminate_child(&mut child).await?;
        let wait_term = tokio::time::timeout(Self::KILL_TIMEOUT, child.wait()).await;
        if let Ok(res) = wait_term {
            return res
                .inspect(|status| info!(?status, "process exited after sigterm"))
                .map(|status| (status, child));
        }

        error!(
            id = ?child.id(),
            kill_timeout = ?Self::KILL_TIMEOUT,
            "second timeout reached, killing process"
        );

        return Err(self.kill_child(&mut child, io::ErrorKind::TimedOut).await);
    }

    async fn kill_child(&mut self, child: &mut Child, reason: io::ErrorKind) -> io::Error {
        info!(%reason, "forcing child exit using kill");

        if let Err(error) = child.kill().await {
            error!(%error, "could not kill child process");
            return error;
        }

        io::Error::new(reason, "forced child exit")
    }

    #[cfg(unix)]
    async fn terminate_child(&mut self, child: &mut Child) -> io::Result<()> {
        use rustix::process::{Pid, Signal};

        let pid = child.id().and_then(|i| Pid::from_raw(i as i32));
        let Some(pid) = pid else {
            // NOTE this should never happen (the Pid::from_raw would need to fail)
            error!("no valid pid of spawned process, killing");
            return Err(self.kill_child(child, io::ErrorKind::Other).await);
        };

        rustix::process::kill_process(pid, Signal::TERM).map_err(io::Error::from)
    }

    #[cfg(windows)]
    async fn terminate_child(&mut self, child: &mut Child) -> io::Result<()> {
        use windows_sys::Win32::System::Console;

        let process_id = child.id();
        let Some(process_id) = process_id else {
            // NOTE this should never happen (the child shouldn't have been awaited)
            error!("no valid process_id of spawned process, killing");
            return Err(self.kill_child(child, io::ErrorKind::Other).await);
        };

        // SAFETY: the values we are passing to the function come from valid sources
        Self::check_windows_call(|| unsafe {
            Console::GenerateConsoleCtrlEvent(Console::CTRL_BREAK_EVENT, process_id)
        })
    }

    #[cfg(windows)]
    fn check_windows_call<F>(function: F) -> io::Result<()>
    where
        F: FnOnce() -> windows_sys::core::BOOL,
    {
        let error = function() == 0;

        if error {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct HookExecutor {
    pub(crate) id: FileTransferId,
    pre: Option<HookDef>,
    post: Option<HookDef>,
}

impl HookExecutor {
    pub(crate) fn with_path_upload<'a>(path: PathBuf, upload: &'a Upload<'a>) -> Self {
        let target_name = upload.source.target_name();
        let target_value = upload.source.target_value();

        Self::new(
            path,
            upload.transfer(),
            target_name,
            target_value.as_deref(),
            upload.pre_hook.as_ref(),
            upload.post_hook.as_ref(),
        )
    }

    pub(crate) fn with_write_handle<'a>(handle: &WriteHandle, download: &'a Download<'a>) -> Self {
        Self::with_path_size_download(
            handle.path().to_path_buf(),
            Some(handle.current_size()),
            download,
        )
    }

    pub(crate) fn with_path_size_download<'a>(
        path: PathBuf,
        size: Option<u64>,
        download: &'a Download<'a>,
    ) -> Self {
        let target_name = download.destination.target_name();
        let target_value = download.destination.target_value();

        let is_first_download = size.is_none_or(|s| s == 0);
        let pre = download
            .pre_hook
            .as_ref()
            // NOTE only execute pre hook if this is the first time we are trying to download this file
            .filter(|_| is_first_download);

        Self::new(
            path,
            download.transfer(),
            target_name,
            target_value.as_deref(),
            pre,
            download.post_hook.as_ref(),
        )
    }

    fn new(
        path: PathBuf,
        transfer: FileTransferId,
        target_name: &str,
        target_value: Option<&Path>,
        pre: Option<&CommandHook<'_>>,
        post: Option<&CommandHook<'_>>,
    ) -> Self {
        let id = transfer.id.to_string();
        let target_value = target_value.unwrap_or(Path::new(""));

        let pre = pre.map(|h| HookDef::new(h).set_env(&id, &path, target_name, target_value));
        let post = post.map(|h| HookDef::new(h).set_env(&id, &path, target_name, target_value));

        Self {
            id: transfer,
            pre,
            post,
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn pre<C>(&mut self, device: &mut C) -> eyre::Result
    where
        C: Client + Send + Sync + 'static,
    {
        let phase = HookPhase::Pre;

        let Some(hook) = self.pre.as_mut() else {
            return Ok(());
        };

        let result = hook.run().await;

        let (status, child) = match result {
            Ok(e) => e,
            Err(ioe) => {
                self.report_io(&ioe, phase, device).await;
                return Err(ioe).wrap_err("encountered io error while running hook");
            }
        };

        self.report_exit(status, phase, device).await;

        self.log_output(child, phase).await;

        Ok(())
    }

    #[instrument(skip_all)]
    pub(crate) async fn post<C>(&mut self, device: &mut C) -> eyre::Result
    where
        C: Client + Send + Sync + 'static,
    {
        let phase = HookPhase::Post;

        let Some(hook) = self.post.as_mut() else {
            return Ok(());
        };

        let result = hook.run().await;

        let (status, child) = match result {
            Ok(e) => e,
            Err(ioe) => {
                self.report_io(&ioe, phase, device).await;
                return Err(ioe).wrap_err("encountered io error while running hook");
            }
        };

        self.report_exit(status, phase, device).await;

        self.log_output(child, phase).await;

        Ok(())
    }

    pub(crate) async fn log_output(&self, mut child: Child, phase: HookPhase) {
        let id = self.id;

        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                if let Err(error) = Self::trace_out(id, phase, stdout).await {
                    error!(%error, "error while reading stdout")
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                if let Err(error) = Self::trace_out(id, phase, stderr).await {
                    error!(%error, "error while reading stderr")
                }
            });
        }
    }

    async fn trace_out<R>(id: FileTransferId, phase: HookPhase, read: R) -> io::Result<()>
    where
        R: AsyncRead + Unpin,
    {
        let reader = BufReader::new(read);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            info!(target:"file-transfer-hook", id = %id.id, %phase, line);
        }

        Ok(())
    }

    async fn report_io<C>(&mut self, error: &io::Error, phase: HookPhase, device: &mut C)
    where
        C: Client + Send + Sync + 'static,
    {
        error!(%error, "io error while running hook");

        if let Err(error) = HookResponse::io_error(self.id, phase, error)
            .send(device)
            .await
        {
            error!(%error, "can't send hook response to astarte");
        }
    }

    async fn report_exit<C>(&mut self, exit: ExitStatus, phase: HookPhase, device: &mut C)
    where
        C: Client + Send + Sync + 'static,
    {
        if let Err(error) = HookResponse::exit_status(self.id, phase, exit)
            .send(device)
            .await
        {
            error!(%error, "can't send hook response to astarte");
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate::eq;
    use uuid::Uuid;

    use super::*;
    use crate::file_transfer::interface::hooks::tests::exit_status_ok;
    use crate::file_transfer::interface::status::TransferDirection;

    fn sample_cmd() -> Vec<String> {
        #[cfg(windows)]
        {
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                "echo hello".to_string(),
            ]
        }
        #[cfg(not(windows))]
        {
            vec!["sh".to_string(), "-c".to_string(), "echo hello".to_string()]
        }
    }

    #[tokio::test]
    async fn execute_pre_hook() {
        let cmd = sample_cmd();
        let hook = CommandHook::new(&cmd, None, None, None)
            .unwrap()
            .expect("hook should be created");

        let id = Uuid::new_v4();
        let file_path = PathBuf::from("/tmp/testfile");

        let mut executor = HookExecutor::new(
            file_path.clone(),
            FileTransferId {
                id,
                direction: TransferDirection::Download,
            },
            "filesystem",
            Some(&file_path),
            Some(&hook),
            None,
        );

        let expected_response =
            HookResponse::exit_status(executor.id, HookPhase::Pre, exit_status_ok());

        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        device
            .expect_send_object()
            .with(
                eq(HookResponse::INTERFACE),
                eq("/response"),
                eq(AstarteObject::try_from(expected_response).unwrap()),
            )
            .once()
            .returning(|_, _, _| Ok(()));

        executor.pre(&mut device).await.unwrap();
        // Since post hook is None, executing post should succeed without sending anything
        executor.post(&mut device).await.unwrap();
    }

    #[tokio::test]
    async fn execute_post_hook() {
        let cmd = sample_cmd();
        let hook = CommandHook::new(&cmd, None, None, None)
            .unwrap()
            .expect("hook should be created");

        let id = Uuid::new_v4();
        let file_path = PathBuf::from("/tmp/testfile");

        let mut executor = HookExecutor::new(
            file_path.clone(),
            FileTransferId {
                id,
                direction: TransferDirection::Download,
            },
            "filesystem",
            Some(&file_path),
            None,
            Some(&hook),
        );

        let expected_response =
            HookResponse::exit_status(executor.id, HookPhase::Post, exit_status_ok());

        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        device
            .expect_send_object()
            .with(
                eq(HookResponse::INTERFACE),
                eq("/response"),
                eq(AstarteObject::try_from(expected_response).unwrap()),
            )
            .once()
            .returning(|_, _, _| Ok(()));

        // Since pre hook is None, executing pre should succeed without sending anything
        executor.pre(&mut device).await.unwrap();
        executor.post(&mut device).await.unwrap();
    }
}
