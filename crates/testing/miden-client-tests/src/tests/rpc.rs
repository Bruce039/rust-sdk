use std::sync::Arc;

use miden_client::account::AccountId;
use miden_client::builder::ClientBuilder;
use miden_client::keystore::FilesystemKeyStore;
use miden_client::rpc::{
    EndpointError,
    GrpcError,
    NodeRpcClient,
    RegisterAccountError,
    RpcEndpoint,
    RpcError,
};
use miden_client::testing::common::create_test_store_path;
use miden_client::testing::mock::MockRpcApi;
use miden_client::{Client, ClientError, ErrorHint, Word};
use miden_client_sqlite_store::ClientBuilderSqliteExt;
use miden_protocol::crypto::rand::RandomCoin;
use miden_testing::MockChain;

use super::ACCOUNT_ID_REGULAR;

const INVITATION_CODE: &str = "Mi-DEN-1234";

fn account_id() -> AccountId {
    AccountId::try_from(ACCOUNT_ID_REGULAR).unwrap()
}

fn rejection(error_kind: GrpcError, endpoint_error: RegisterAccountError) -> RpcError {
    RpcError::RequestError {
        endpoint: RpcEndpoint::RegisterAccount,
        error_kind,
        endpoint_error: Some(endpoint_error.into()),
        source: None,
    }
}

/// The node matches the exact text, so trimming or case folding on the way out would turn a valid
/// code into an unknown one.
#[tokio::test]
async fn register_account_sends_the_code_unchanged() {
    let rpc_api = MockRpcApi::new(MockChain::new());

    rpc_api.register_account(INVITATION_CODE, account_id()).await.unwrap();

    assert_eq!(
        rpc_api.registered_invitation_code(account_id()).as_deref(),
        Some(INVITATION_CODE)
    );
}

#[tokio::test]
async fn register_account_reports_an_unknown_code() {
    let rpc_api = MockRpcApi::new(MockChain::new());
    rpc_api.fail_next_call(
        RpcEndpoint::RegisterAccount,
        rejection(GrpcError::NotFound, RegisterAccountError::InvitationNotFound),
    );

    let error = rpc_api.register_account(INVITATION_CODE, account_id()).await.unwrap_err();

    assert!(matches!(
        error.endpoint_error(),
        Some(EndpointError::RegisterAccount(RegisterAccountError::InvitationNotFound))
    ));
    assert!(rpc_api.registered_invitation_code(account_id()).is_none());
}

#[tokio::test]
async fn register_account_reports_a_consumed_code() {
    let rpc_api = MockRpcApi::new(MockChain::new());
    rpc_api.fail_next_call(
        RpcEndpoint::RegisterAccount,
        rejection(GrpcError::AlreadyExists, RegisterAccountError::AlreadyRegistered),
    );

    let error = rpc_api.register_account(INVITATION_CODE, account_id()).await.unwrap_err();

    assert!(matches!(
        error.endpoint_error(),
        Some(EndpointError::RegisterAccount(RegisterAccountError::AlreadyRegistered))
    ));
}

/// The node treats a repeat with the same code and account as a no-op, which is what lets the
/// endpoint be retried after a lost response.
#[tokio::test]
async fn register_account_is_retryable() {
    let rpc_api = MockRpcApi::new(MockChain::new());

    rpc_api.register_account(INVITATION_CODE, account_id()).await.unwrap();
    rpc_api.register_account(INVITATION_CODE, account_id()).await.unwrap();

    assert_eq!(
        rpc_api.registered_invitation_code(account_id()).as_deref(),
        Some(INVITATION_CODE)
    );
}

// CLIENT METHOD
// ================================================================================================

/// Builds a client whose RPC layer is `rpc_api`.
///
/// Registration does not read or write the store, so the client needs no genesis state.
async fn client_with_rpc(rpc_api: Arc<MockRpcApi>) -> Client<FilesystemKeyStore> {
    let keystore = FilesystemKeyStore::new(std::env::temp_dir()).unwrap();

    ClientBuilder::new()
        .rpc(rpc_api)
        .rng(Box::new(RandomCoin::new(Word::from([0xfeeu32, 1, 2, 3]))))
        .sqlite_store(create_test_store_path())
        .authenticator(Arc::new(keystore))
        .tx_discard_delta(None)
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn client_register_account_forwards_the_code_unchanged() {
    let rpc_api = Arc::new(MockRpcApi::new(MockChain::new()));
    let client = client_with_rpc(rpc_api.clone()).await;

    client.register_account(INVITATION_CODE, account_id()).await.unwrap();

    assert_eq!(
        rpc_api.registered_invitation_code(account_id()).as_deref(),
        Some(INVITATION_CODE)
    );
}

#[tokio::test]
async fn client_register_account_reports_an_unknown_code() {
    let rpc_api = Arc::new(MockRpcApi::new(MockChain::new()));
    rpc_api.fail_next_call(
        RpcEndpoint::RegisterAccount,
        rejection(GrpcError::NotFound, RegisterAccountError::InvitationNotFound),
    );
    let client = client_with_rpc(rpc_api.clone()).await;

    let error = client.register_account(INVITATION_CODE, account_id()).await.unwrap_err();

    let ClientError::RpcError(rpc_error) = &error else {
        panic!("expected an RPC error, got {error:?}");
    };
    assert!(matches!(
        rpc_error.endpoint_error(),
        Some(EndpointError::RegisterAccount(RegisterAccountError::InvitationNotFound))
    ));
    assert!(rpc_api.registered_invitation_code(account_id()).is_none());
}

/// The store is never touched, so a rejected registration leaves no trace and the same account can
/// be registered again with another code.
#[tokio::test]
async fn client_register_account_can_be_retried_after_a_rejection() {
    let rpc_api = Arc::new(MockRpcApi::new(MockChain::new()));
    rpc_api.fail_next_call(
        RpcEndpoint::RegisterAccount,
        rejection(GrpcError::NotFound, RegisterAccountError::InvitationNotFound),
    );
    let client = client_with_rpc(rpc_api.clone()).await;

    client.register_account("wrong-code", account_id()).await.unwrap_err();
    client.register_account(INVITATION_CODE, account_id()).await.unwrap();

    assert_eq!(
        rpc_api.registered_invitation_code(account_id()).as_deref(),
        Some(INVITATION_CODE)
    );
}
