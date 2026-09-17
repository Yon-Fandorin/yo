use yo_core::{AccountCapacitySnapshot, AccountId, ApiCredential, ProviderId};

mod decode;
mod error;
mod http;
mod model;

pub use error::{QwenCloudCapacityError, QwenCloudCapacityFailureKind};
pub use model::QwenCloudProviderData;

/// QwenCloud의 브라우저 콘솔 세션 Cookie가 용량 조회에 사용할 수 있는지 검증합니다.
pub fn validate_account_session(
    cookie: ApiCredential,
) -> Result<ApiCredential, QwenCloudCapacityError> {
    http::validate_account_session(cookie)
}

/// 인증된 콘솔 세션을 통해 QwenCloud Personal Token Plan 용량을 읽습니다.
///
/// 추론용 `sk-sp-*` 키는 콘솔 gateway를 인증할 수 없습니다. 호출자는 개인 자격 증명
/// 저장소에서 캡처한 계정 세션 비밀값을 그대로 전달해야 합니다.
pub fn read_account_capacity(
    provider: &ProviderId,
    account: &AccountId,
    cookie: &ApiCredential,
) -> Result<(AccountCapacitySnapshot, QwenCloudProviderData), QwenCloudCapacityError> {
    http::read_account_capacity(provider, account, cookie)
}

#[cfg(test)]
mod tests;
