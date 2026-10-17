use super::params::AlpacaParseError;
use crate::ASCOMError;
use crate::api::DeviceType;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum Error {
    #[error("Device {ty:?}[{device_number}] not found")]
    UnknownDeviceNumber {
        ty: DeviceType,
        device_number: usize,
    },
    #[error("Unknown action {device_type:?}::{action}")]
    UnknownAction {
        device_type: DeviceType,
        action: String,
    },
    #[error("Missing parameter {name:?}")]
    MissingParameter { name: &'static str },
    #[error("Couldn't parse parameter {name:?}: {err:#}")]
    BadParameter {
        name: &'static str,
        #[source]
        err: AlpacaParseError,
    },
    #[error(transparent)]
    Ascom(#[from] ASCOMError),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        // A device number the server does not have and an action the device
        // does not know are both "the device couldn't interpret the request":
        // the API Reference's status-code examples answer "Camera device 1
        // does not exist" and "'CanSlew' is not a valid Camera command" with
        // 400, alongside a missing or malformed parameter.
        let code = match self {
            Self::UnknownDeviceNumber { .. }
            | Self::UnknownAction { .. }
            | Self::MissingParameter { .. }
            | Self::BadParameter { .. } => StatusCode::BAD_REQUEST,
            Self::Ascom(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, format!("{self:#}")).into_response()
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
