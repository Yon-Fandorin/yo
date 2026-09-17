//! 입력 reference 값과 모델 투영을 위한 안정적인 파사드.

mod builder;
mod error;
mod model;
mod model_projection;
mod validation;

pub use error::UserInputError;
pub use model::{InputReference, ResolvedSkill, UserInput};
