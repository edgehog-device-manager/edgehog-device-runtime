use std::io;

use astarte_device_sdk::{AstarteData, FromEvent, IntoAstarteObject, aggregate::AstarteObject};
use eyre::bail;
use tracing::instrument;

use crate::{
    file_transfer::errno,
    storage::{StorageJobTag, request::FileStorageId},
};

#[derive(Debug, Clone, PartialEq, FromEvent, IntoAstarteObject)]
#[from_event(
    interface = "io.edgehog.devicemanager.storage.DeleteFile",
    aggregation = "object",
    path = "/request",
    rename_all = "camelCase"
)]
#[astarte_object(rename_all = "camelCase")]
pub struct DeleteFile {
    pub(crate) id: String,
    pub(crate) file_id: String,
    pub(crate) force: bool,
}

impl DeleteFile {
    pub(crate) const RESPONSE_TYPE: ActionType = ActionType::Delete;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ActionType {
    Delete,
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Delete => f.write_str("delete"),
        }
    }
}

impl From<ActionType> for AstarteData {
    fn from(value: ActionType) -> Self {
        AstarteData::String(value.to_string())
    }
}

impl TryFrom<StorageJobTag> for ActionType {
    type Error = eyre::Error;

    fn try_from(value: StorageJobTag) -> Result<Self, Self::Error> {
        match value {
            StorageJobTag::CleanUp => bail!("cleanup job shouldn't send reseponses"),
            StorageJobTag::Delete => Ok(ActionType::Delete),
        }
    }
}

impl From<ActionType> for StorageJobTag {
    fn from(value: ActionType) -> Self {
        match value {
            ActionType::Delete => StorageJobTag::Delete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
pub(crate) struct FileStorageResponse {
    id: String,
    #[astarte_object(rename = "type")]
    ty: ActionType,
    code: i32,
    messages: Vec<String>,
}

impl FileStorageResponse {
    const INTERFACE: &str = "io.edgehog.devicemanager.storage.Response";

    pub(crate) fn success(store: FileStorageId) -> Self {
        Self {
            id: store.id.to_string(),
            ty: store.ty,
            code: errno::OK,
            messages: Vec::new(),
        }
    }

    fn to_backtrace(report: eyre::Report) -> Vec<String> {
        let mut backtrace = vec![report.to_string()];

        backtrace.extend(report.chain().map(|e| e.to_string()));

        backtrace
    }

    pub(crate) fn validation_error(id: &str, ty: ActionType, report: eyre::Report) -> Self {
        Self {
            id: id.to_string(),
            ty,
            code: errno::to_errno(io::ErrorKind::InvalidInput),
            messages: Self::to_backtrace(report),
        }
    }

    pub(crate) fn busy_error(id: &str, ty: ActionType) -> Self {
        Self {
            id: id.to_string(),
            ty,
            code: errno::to_errno(io::ErrorKind::ResourceBusy),
            messages: vec!["file transfer request can't be handled currently".to_string()],
        }
    }

    pub(crate) fn runtime_error(store: FileStorageId, report: eyre::Report) -> Self {
        Self {
            id: store.id.to_string(),
            ty: store.ty,
            code: errno::to_errno(io::ErrorKind::Other),
            messages: Self::to_backtrace(report),
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn send<C>(self, device: &mut C) -> eyre::Result<()>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        device
            .send_object(Self::INTERFACE, "/request", AstarteObject::try_from(self)?)
            .await
            .map_err(eyre::Error::from)
    }
}
