use std::error::Error;
use std::io;

pub type AppError = Box<dyn Error + Send + Sync>;
pub type AppResult<T> = Result<T, AppError>;

pub fn boxed_error(message: impl Into<String>) -> AppError {
    io::Error::other(message.into()).into()
}
