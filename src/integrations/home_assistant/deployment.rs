use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use opentelemetry::{KeyValue, global};
use russh::{client, keys::PublicKey};
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpError},
    protocol::{FileAttributes, OpenFlags, StatusCode},
};
use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::Semaphore,
    time::timeout,
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

#[cfg(test)]
use std::path::Path;

use crate::{config::HomeAssistantSshConfig, tool_error::ToolError};

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const DOMAIN: &str = "smarthome_mcp";
const CUSTOM_COMPONENTS: &str = "custom_components";
const ACTIVE_NAME: &str = "smarthome_mcp";
const TRANSACTION_NAME: &str = ".smarthome_mcp-deploy";
const STAGING_NAME: &str = "staging";
const BACKUP_NAME: &str = "backup";
const LOCK_NAME: &str = "lock";
const CLAIM_NAME: &str = "lock.claim";
const JOURNAL_NAME: &str = "journal.json";
const MAX_ENTRIES: usize = 32;
const MAX_FILE_SIZE: u64 = 1024 * 1024;
const MAX_TOTAL_SIZE: u64 = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 4;
const MAX_SFTP_PACKET: usize = 1024 * 1024;
const SFTP_CHUNK: u32 = 32 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const LOCK_STALE_AFTER_SECS: u64 = 300;
const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(240);
const LOCK_MAX_SIZE: u64 = 256;
const OWNER_TOKEN_BYTES: usize = 32;

struct EmbeddedFile {
    path: &'static str,
    bytes: &'static [u8],
}

const FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        path: "__init__.py",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/__init__.py"),
    },
    EmbeddedFile {
        path: "brand/icon.png",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/brand/icon.png"),
    },
    EmbeddedFile {
        path: "config_flow.py",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/config_flow.py"),
    },
    EmbeddedFile {
        path: "const.py",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/const.py"),
    },
    EmbeddedFile {
        path: "manifest.json",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/manifest.json"),
    },
    EmbeddedFile {
        path: "strings.json",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/strings.json"),
    },
    EmbeddedFile {
        path: "translations/en.json",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/translations/en.json"),
    },
    EmbeddedFile {
        path: "websocket_api.py",
        bytes: include_bytes!("../../../custom_components/smarthome_mcp/websocket_api.py"),
    },
];

type RemoteFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DeployError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    kind: NodeKind,
    size: u64,
    mode: u32,
}

trait RemoteFs: Send + Sync {
    fn lstat<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Option<Node>>;
    fn read_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<String>>;
    fn read<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<u8>>;
    fn mkdir<'a>(&'a self, path: &'a str, mode: u32) -> RemoteFuture<'a, ()>;
    fn write_exclusive<'a>(
        &'a self,
        path: &'a str,
        bytes: &'a [u8],
        mode: u32,
    ) -> RemoteFuture<'a, ()>;
    fn rename<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteFuture<'a, ()>;
    fn remove_file<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()>;
    fn remove_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()>;
}

trait RemoteConnector: Send + Sync {
    fn connect(&self) -> RemoteFuture<'_, Arc<dyn RemoteFs>>;
}

#[derive(Clone)]
pub struct ComponentDeployer {
    connector: Arc<dyn RemoteConnector>,
    permit: Arc<Semaphore>,
    config_root: Arc<str>,
    #[cfg(test)]
    test_result: Option<Arc<TestDeployResult>>,
}

#[cfg(test)]
struct TestDeployResult {
    calls: Arc<AtomicUsize>,
    result: Result<DeployOutput, DeployError>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeployOutput {
    pub action: &'static str,
    pub operation: &'static str,
    pub changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    pub installed_version: String,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployError {
    InvalidArguments,
    CapacityExhausted,
    Timeout,
    HostKeyMismatch,
    AuthenticationFailed,
    Unavailable,
    UnsafeState,
    VerificationFailed,
    RollbackFailed,
    CleanupIncomplete,
}

impl DeployError {
    pub fn into_tool_error(self) -> ToolError {
        let (code, message, retryable) = match self {
            Self::InvalidArguments => (
                "invalid_arguments",
                "The deploy arguments are invalid.",
                false,
            ),
            Self::CapacityExhausted => (
                "capacity_exhausted",
                "A component deployment is already in progress.",
                true,
            ),
            Self::Timeout => ("timeout", "The component deployment timed out.", true),
            Self::HostKeyMismatch => (
                "host_key_mismatch",
                "The deployment target host identity did not match its configured key.",
                false,
            ),
            Self::AuthenticationFailed => (
                "authentication_failed",
                "The deployment target rejected its configured credentials.",
                false,
            ),
            Self::Unavailable => (
                "deployment_unavailable",
                "The component deployment target is unavailable.",
                true,
            ),
            Self::UnsafeState => (
                "unsafe_deployment_state",
                "The installed component or transaction state is not safe to modify.",
                false,
            ),
            Self::VerificationFailed => (
                "deployment_verification_failed",
                "The staged component did not pass content verification.",
                false,
            ),
            Self::RollbackFailed => (
                "deployment_rollback_failed",
                "The component deployment failed and automatic recovery could not complete.",
                false,
            ),
            Self::CleanupIncomplete => (
                "deployment_cleanup_incomplete",
                "The deployment completed with recovery-required cleanup still pending.",
                true,
            ),
        };
        ToolError::new(code, message, retryable)
    }
}

impl ComponentDeployer {
    pub fn production(config: &HomeAssistantSshConfig) -> Result<Self, String> {
        validate_embedded().map_err(|_| "invalid embedded component package".to_owned())?;
        Ok(Self {
            connector: Arc::new(NativeConnector::new(config)?),
            permit: Arc::new(Semaphore::new(1)),
            config_root: Arc::from(config.config_root.as_str()),
            #[cfg(test)]
            test_result: None,
        })
    }

    pub async fn deploy(&self) -> Result<DeployOutput, DeployError> {
        #[cfg(test)]
        if let Some(test_result) = &self.test_result {
            test_result.calls.fetch_add(1, AtomicOrdering::Relaxed);
            return test_result.result.clone();
        }
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| DeployError::CapacityExhausted)?;
        let connector = self.connector.clone();
        let config_root = self.config_root.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut metrics = DeploymentMetricsGuard::new();
            let result = async {
                let fs = connector.connect().await?;
                run_transaction(fs.as_ref(), &config_root).await
            }
            .await;
            metrics.finish(deploy_outcome(&result));
            result
        })
        .await
        .map_err(|_| DeployError::Unavailable)?
    }

    #[cfg(test)]
    fn new(connector: Arc<dyn RemoteConnector>) -> Self {
        Self {
            connector,
            permit: Arc::new(Semaphore::new(1)),
            config_root: Arc::from("/config"),
            test_result: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        struct UnavailableConnector;
        impl RemoteConnector for UnavailableConnector {
            fn connect(&self) -> RemoteFuture<'_, Arc<dyn RemoteFs>> {
                Box::pin(async { Err(DeployError::Unavailable) })
            }
        }
        Self::new(Arc::new(UnavailableConnector))
    }

    #[cfg(test)]
    pub(crate) fn successful_for_test(calls: Arc<AtomicUsize>) -> Self {
        let mut deployer = Self::unavailable();
        deployer.test_result = Some(Arc::new(TestDeployResult {
            calls,
            result: Ok(output("install", true, None)),
        }));
        deployer
    }
}

struct DeploymentMetrics {
    requests: opentelemetry::metrics::Counter<u64>,
    duration: opentelemetry::metrics::Histogram<f64>,
    in_flight: opentelemetry::metrics::UpDownCounter<i64>,
}

fn deployment_metrics() -> &'static DeploymentMetrics {
    static METRICS: OnceLock<DeploymentMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = global::meter("smarthome_mcp.component_deployment");
        DeploymentMetrics {
            requests: meter
                .u64_counter("smarthome_mcp.component_deployment.requests")
                .build(),
            duration: meter
                .f64_histogram("smarthome_mcp.component_deployment.duration")
                .with_unit("s")
                .build(),
            in_flight: meter
                .i64_up_down_counter("smarthome_mcp.component_deployment.in_flight")
                .build(),
        }
    })
}

struct DeploymentMetricsGuard {
    started: Instant,
    finished: bool,
}

impl DeploymentMetricsGuard {
    fn new() -> Self {
        deployment_metrics().in_flight.add(1, &[]);
        Self {
            started: Instant::now(),
            finished: false,
        }
    }

    fn finish(&mut self, outcome: &'static str) {
        if self.finished {
            return;
        }
        let attributes = [KeyValue::new("outcome", outcome)];
        let metrics = deployment_metrics();
        metrics.in_flight.add(-1, &[]);
        metrics.requests.add(1, &attributes);
        metrics
            .duration
            .record(self.started.elapsed().as_secs_f64(), &attributes);
        self.finished = true;
    }
}

impl Drop for DeploymentMetricsGuard {
    fn drop(&mut self) {
        self.finish("cancelled");
    }
}

fn deploy_outcome(result: &Result<DeployOutput, DeployError>) -> &'static str {
    match result {
        Ok(_) => "success",
        Err(DeployError::InvalidArguments) => "invalid_arguments",
        Err(DeployError::CapacityExhausted) => "capacity_exhausted",
        Err(DeployError::Timeout) => "timeout",
        Err(DeployError::HostKeyMismatch) => "host_key_mismatch",
        Err(DeployError::AuthenticationFailed) => "authentication_failed",
        Err(DeployError::Unavailable) => "unavailable",
        Err(DeployError::UnsafeState) => "unsafe_state",
        Err(DeployError::VerificationFailed) => "verification_failed",
        Err(DeployError::RollbackFailed) => "rollback_failed",
        Err(DeployError::CleanupIncomplete) => "cleanup_incomplete",
    }
}

#[derive(Deserialize)]
struct IntegrationManifest {
    domain: String,
    version: String,
}

fn validate_embedded() -> Result<(), DeployError> {
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for file in FILES {
        if !valid_relative_path(file.path)
            || !paths.insert(file.path)
            || file.bytes.is_empty()
            || file.bytes.len() as u64 > MAX_FILE_SIZE
        {
            return Err(DeployError::VerificationFailed);
        }
        total += file.bytes.len() as u64;
    }
    if total > MAX_TOTAL_SIZE {
        return Err(DeployError::VerificationFailed);
    }
    let manifest: IntegrationManifest = serde_json::from_slice(
        FILES
            .iter()
            .find(|file| file.path == "manifest.json")
            .ok_or(DeployError::VerificationFailed)?
            .bytes,
    )
    .map_err(|_| DeployError::VerificationFailed)?;
    if manifest.domain != DOMAIN || manifest.version != PACKAGE_VERSION {
        return Err(DeployError::VerificationFailed);
    }
    Ok(())
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 255
        && !path.starts_with('.')
        && path.split('/').count() <= MAX_DEPTH
        && path.split('/').all(|part| {
            !part.is_empty()
                && !matches!(part, "." | "..")
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
}

struct Paths {
    config: String,
    components: String,
    active: String,
    transaction: String,
    staging: String,
    backup: String,
    lock: String,
    claim: String,
    journal: String,
}

impl Paths {
    fn new(root: &str) -> Self {
        let components = join(root, CUSTOM_COMPONENTS);
        let transaction = join(root, TRANSACTION_NAME);
        Self {
            config: root.to_owned(),
            active: join(&components, ACTIVE_NAME),
            staging: join(&transaction, STAGING_NAME),
            backup: join(&transaction, BACKUP_NAME),
            lock: join(&transaction, LOCK_NAME),
            claim: join(&transaction, CLAIM_NAME),
            journal: join(&transaction, JOURNAL_NAME),
            components,
            transaction,
        }
    }
}

fn join(parent: &str, child: &str) -> String {
    format!("{}/{child}", parent.trim_end_matches('/'))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LockFile {
    schema: u8,
    owner: String,
    lease_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Install,
    Update,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u8,
    operation: Operation,
    target_version: String,
}

async fn run_transaction(fs: &dyn RemoteFs, root: &str) -> Result<DeployOutput, DeployError> {
    run_transaction_with_timeout(fs, root, TRANSACTION_TIMEOUT).await
}

async fn run_transaction_with_timeout(
    fs: &dyn RemoteFs,
    root: &str,
    transaction_timeout: Duration,
) -> Result<DeployOutput, DeployError> {
    validate_embedded()?;
    let paths = Paths::new(root);
    require_kind(fs, &paths.config, NodeKind::Directory).await?;
    ensure_directory(fs, &paths.transaction).await?;
    let owned = acquire_lock(fs, &paths).await?;

    let result = timeout(transaction_timeout, async {
        ensure_directory(&owned, &paths.components).await?;
        run_locked(&owned, &paths).await
    })
    .await
    .unwrap_or(Err(DeployError::Timeout));
    let unlock = owned.unlock().await;
    match (result, unlock) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (_, Err(_)) => Err(DeployError::CleanupIncomplete),
    }
}

struct ObservedLock {
    metadata: LockFile,
    bytes: Vec<u8>,
}

struct OwnedFs<'a> {
    inner: &'a dyn RemoteFs,
    paths: &'a Paths,
    lock_bytes: Vec<u8>,
}

impl OwnedFs<'_> {
    async fn verify_owner(&self) -> Result<(), DeployError> {
        verify_exact_private_file(self.inner, &self.paths.lock, &self.lock_bytes).await?;
        if self.inner.lstat(&self.paths.claim).await?.is_some() {
            return Err(DeployError::UnsafeState);
        }
        Ok(())
    }

    async fn unlock(&self) -> Result<(), DeployError> {
        self.verify_owner().await?;
        self.inner.remove_file(&self.paths.lock).await
    }
}

impl RemoteFs for OwnedFs<'_> {
    fn lstat<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Option<Node>> {
        self.inner.lstat(path)
    }

    fn read_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<String>> {
        self.inner.read_dir(path)
    }

    fn read<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<u8>> {
        self.inner.read(path)
    }

    fn mkdir<'a>(&'a self, path: &'a str, mode: u32) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.verify_owner().await?;
            self.inner.mkdir(path, mode).await
        })
    }

    fn write_exclusive<'a>(
        &'a self,
        path: &'a str,
        bytes: &'a [u8],
        mode: u32,
    ) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.verify_owner().await?;
            self.inner.write_exclusive(path, bytes, mode).await
        })
    }

    fn rename<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.verify_owner().await?;
            self.inner.rename(from, to).await
        })
    }

    fn remove_file<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.verify_owner().await?;
            self.inner.remove_file(path).await
        })
    }

    fn remove_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.verify_owner().await?;
            self.inner.remove_dir(path).await
        })
    }
}

async fn acquire_lock<'a>(
    fs: &'a dyn RemoteFs,
    paths: &'a Paths,
) -> Result<OwnedFs<'a>, DeployError> {
    let now = unix_time()?;
    let claim = read_lock_file(fs, &paths.claim, now).await?;
    let lock = read_lock_file(fs, &paths.lock, now).await?;
    if claim.is_some() && lock.is_some() {
        return Err(DeployError::UnsafeState);
    }

    let owner = owner_token()?;
    let lock_bytes = serde_json::to_vec(&LockFile {
        schema: 1,
        owner,
        lease_at: now,
    })
    .map_err(|_| DeployError::UnsafeState)?;

    match (claim, lock) {
        (Some(claim), None) => {
            if !is_stale(&claim.metadata, now) {
                return Err(DeployError::CapacityExhausted);
            }
            verify_exact_private_file(fs, &paths.claim, &claim.bytes).await?;
            create_owned_lock(fs, paths, &lock_bytes).await?;
            verify_exact_private_file(fs, &paths.lock, &lock_bytes).await?;
            fs.remove_file(&paths.claim).await?;
        }
        (None, Some(lock)) => {
            if !is_stale(&lock.metadata, now) {
                return Err(DeployError::CapacityExhausted);
            }
            fs.rename(&paths.lock, &paths.claim)
                .await
                .map_err(|_| DeployError::CapacityExhausted)?;
            verify_exact_private_file(fs, &paths.claim, &lock.bytes).await?;
            if fs.lstat(&paths.lock).await?.is_some() {
                return Err(DeployError::UnsafeState);
            }
            create_owned_lock(fs, paths, &lock_bytes).await?;
            verify_exact_private_file(fs, &paths.claim, &lock.bytes).await?;
            verify_exact_private_file(fs, &paths.lock, &lock_bytes).await?;
            fs.remove_file(&paths.claim).await?;
        }
        (None, None) => {
            if fs.lstat(&paths.claim).await?.is_some() {
                return Err(DeployError::CapacityExhausted);
            }
            create_owned_lock(fs, paths, &lock_bytes).await?;
        }
        (Some(_), Some(_)) => unreachable!("handled above"),
    }

    let owned = OwnedFs {
        inner: fs,
        paths,
        lock_bytes,
    };
    owned.verify_owner().await?;
    Ok(owned)
}

async fn create_owned_lock(
    fs: &dyn RemoteFs,
    paths: &Paths,
    bytes: &[u8],
) -> Result<(), DeployError> {
    fs.write_exclusive(&paths.lock, bytes, 0o600)
        .await
        .map_err(|error| match error {
            DeployError::UnsafeState => DeployError::CapacityExhausted,
            other => other,
        })
}

async fn read_lock_file(
    fs: &dyn RemoteFs,
    path: &str,
    now: u64,
) -> Result<Option<ObservedLock>, DeployError> {
    let Some(node) = fs.lstat(path).await? else {
        return Ok(None);
    };
    if node.kind != NodeKind::File || node.mode != 0o600 || node.size > LOCK_MAX_SIZE {
        return Err(DeployError::UnsafeState);
    }
    let bytes = fs.read(path).await?;
    if bytes.len() as u64 != node.size {
        return Err(DeployError::UnsafeState);
    }
    let metadata: LockFile =
        serde_json::from_slice(&bytes).map_err(|_| DeployError::UnsafeState)?;
    if metadata.schema != 1 || metadata.lease_at > now || !valid_owner_token(&metadata.owner) {
        return Err(DeployError::UnsafeState);
    }
    Ok(Some(ObservedLock { metadata, bytes }))
}

async fn verify_exact_private_file(
    fs: &dyn RemoteFs,
    path: &str,
    expected: &[u8],
) -> Result<(), DeployError> {
    match fs.lstat(path).await? {
        Some(node)
            if node.kind == NodeKind::File
                && node.mode == 0o600
                && node.size == expected.len() as u64 => {}
        _ => return Err(DeployError::UnsafeState),
    }
    if fs.read(path).await? != expected {
        return Err(DeployError::UnsafeState);
    }
    Ok(())
}

fn owner_token() -> Result<String, DeployError> {
    let mut bytes = [0_u8; OWNER_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| DeployError::Unavailable)?;
    let mut token = String::with_capacity(OWNER_TOKEN_BYTES * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(token, "{byte:02x}").map_err(|_| DeployError::Unavailable)?;
    }
    Ok(token)
}

fn valid_owner_token(owner: &str) -> bool {
    owner.len() == OWNER_TOKEN_BYTES * 2
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_stale(lock: &LockFile, now: u64) -> bool {
    now.saturating_sub(lock.lease_at) > LOCK_STALE_AFTER_SECS
}

fn unix_time() -> Result<u64, DeployError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| DeployError::Unavailable)
}

async fn run_locked(fs: &dyn RemoteFs, paths: &Paths) -> Result<DeployOutput, DeployError> {
    reconcile(fs, paths).await?;
    let installed = inspect_tree(fs, &paths.active, TreePolicy::Recognized).await?;
    let target = Version::parse(PACKAGE_VERSION).map_err(|_| DeployError::VerificationFailed)?;
    let (operation, previous_version) = match installed {
        None => (Operation::Install, None),
        Some(tree) => {
            let manifest = installed_manifest(&tree.files)?;
            let current =
                Version::parse(&manifest.version).map_err(|_| DeployError::UnsafeState)?;
            match current.cmp(&target) {
                std::cmp::Ordering::Greater => return Err(DeployError::UnsafeState),
                std::cmp::Ordering::Equal if manifest.version != PACKAGE_VERSION => {
                    return Err(DeployError::UnsafeState);
                }
                std::cmp::Ordering::Equal => {
                    verify_exact(&tree).map_err(|_| DeployError::UnsafeState)?;
                    return Ok(output("noop", false, None));
                }
                std::cmp::Ordering::Less => {}
            }
            (Operation::Update, Some(current.to_string()))
        }
    };

    if let Err(error) = stage(fs, paths).await {
        if fs.lstat(&paths.staging).await?.is_some() {
            remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
        }
        return Err(error);
    }
    let journal = Journal {
        schema: 1,
        operation,
        target_version: PACKAGE_VERSION.to_owned(),
    };
    write_journal(fs, paths, &journal).await?;
    let commit = commit(fs, paths, &journal).await;
    match commit {
        Ok(()) => Ok(output(
            match operation {
                Operation::Install => "install",
                Operation::Update => "update",
            },
            true,
            previous_version,
        )),
        Err(error) => Err(error),
    }
}

fn output(
    operation: &'static str,
    changed: bool,
    previous_version: Option<String>,
) -> DeployOutput {
    DeployOutput {
        action: "smarthome_mcp.deploy",
        operation,
        changed,
        previous_version,
        installed_version: PACKAGE_VERSION.to_owned(),
        restart_required: changed,
    }
}

async fn stage(fs: &dyn RemoteFs, paths: &Paths) -> Result<(), DeployError> {
    if fs.lstat(&paths.staging).await?.is_some() {
        remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
    }
    fs.mkdir(&paths.staging, 0o755).await?;
    for directory in expected_directories() {
        fs.mkdir(&join(&paths.staging, directory), 0o755).await?;
    }
    for file in FILES {
        fs.write_exclusive(&join(&paths.staging, file.path), file.bytes, 0o644)
            .await?;
    }
    let tree = inspect_tree(fs, &paths.staging, TreePolicy::CurrentExact)
        .await?
        .ok_or(DeployError::VerificationFailed)?;
    verify_exact(&tree)
}

async fn commit(fs: &dyn RemoteFs, paths: &Paths, journal: &Journal) -> Result<(), DeployError> {
    if journal.operation == Operation::Update {
        if fs.lstat(&paths.backup).await?.is_some() {
            remove_package_tree(fs, &paths.backup, TreePolicy::Recognized).await?;
        }
        fs.rename(&paths.active, &paths.backup).await?;
    }

    if let Err(error) = fs.rename(&paths.staging, &paths.active).await {
        rollback(fs, paths, journal.operation).await?;
        return Err(error);
    }
    fs.remove_file(&paths.journal)
        .await
        .map_err(|_| DeployError::CleanupIncomplete)?;
    Ok(())
}

async fn rollback(
    fs: &dyn RemoteFs,
    paths: &Paths,
    operation: Operation,
) -> Result<(), DeployError> {
    let result: Result<(), DeployError> = async {
        if fs.lstat(&paths.active).await?.is_some() {
            fs.rename(&paths.active, &paths.staging).await?;
        }
        if operation == Operation::Update {
            let backup = inspect_tree(fs, &paths.backup, TreePolicy::Recognized)
                .await?
                .ok_or(DeployError::UnsafeState)?;
            validate_recognized_version(&backup.files)?;
            fs.rename(&paths.backup, &paths.active).await?;
        }
        if fs.lstat(&paths.staging).await?.is_some() {
            remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
        }
        if fs.lstat(&paths.journal).await?.is_some() {
            fs.remove_file(&paths.journal).await?;
        }
        Ok(())
    }
    .await;
    result.map_err(|_| DeployError::RollbackFailed)
}

async fn reconcile(fs: &dyn RemoteFs, paths: &Paths) -> Result<(), DeployError> {
    let journal = match fs.lstat(&paths.journal).await? {
        None => None,
        Some(node) if node.kind == NodeKind::File && node.mode == 0o600 && node.size <= 512 => {
            let bytes = fs.read(&paths.journal).await?;
            if bytes.len() as u64 != node.size {
                return Err(DeployError::UnsafeState);
            }
            let journal: Journal =
                serde_json::from_slice(&bytes).map_err(|_| DeployError::UnsafeState)?;
            if journal.schema != 1 || journal.target_version != PACKAGE_VERSION {
                return Err(DeployError::UnsafeState);
            }
            Some(journal)
        }
        Some(_) => return Err(DeployError::UnsafeState),
    };

    let active = fs.lstat(&paths.active).await?.is_some();
    let staging = fs.lstat(&paths.staging).await?.is_some();
    let backup = fs.lstat(&paths.backup).await?.is_some();
    if backup {
        let tree = inspect_tree(fs, &paths.backup, TreePolicy::Recognized)
            .await?
            .ok_or(DeployError::UnsafeState)?;
        validate_recognized_version(&tree.files)?;
    }

    if journal.is_none() {
        if staging {
            remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
        }
        return Ok(());
    }
    let journal = journal.expect("checked above");
    match (journal.operation, active, staging, backup) {
        (Operation::Install, false, true, false) => {
            remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
        }
        (Operation::Install, true, false, false) => {
            let tree = inspect_tree(fs, &paths.active, TreePolicy::Recognized)
                .await?
                .ok_or(DeployError::UnsafeState)?;
            verify_exact(&tree)?;
        }
        (Operation::Update, true, true, _) => {
            remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
        }
        (Operation::Update, false, true, true) => {
            fs.rename(&paths.backup, &paths.active)
                .await
                .map_err(|_| DeployError::RollbackFailed)?;
            remove_package_tree(fs, &paths.staging, TreePolicy::CurrentPartial).await?;
        }
        (Operation::Update, true, false, true) => {
            let tree = inspect_tree(fs, &paths.active, TreePolicy::Recognized)
                .await?
                .ok_or(DeployError::UnsafeState)?;
            verify_exact(&tree)?;
        }
        _ => return Err(DeployError::UnsafeState),
    }
    fs.remove_file(&paths.journal)
        .await
        .map_err(|_| DeployError::CleanupIncomplete)
}

async fn write_journal(
    fs: &dyn RemoteFs,
    paths: &Paths,
    journal: &Journal,
) -> Result<(), DeployError> {
    let bytes = serde_json::to_vec(journal).map_err(|_| DeployError::UnsafeState)?;
    fs.write_exclusive(&paths.journal, &bytes, 0o600).await?;
    verify_exact_private_file(fs, &paths.journal, &bytes).await
}

async fn ensure_directory(fs: &dyn RemoteFs, path: &str) -> Result<(), DeployError> {
    match fs.lstat(path).await? {
        None => fs.mkdir(path, 0o755).await,
        Some(node) if node.kind == NodeKind::Directory && node.mode == 0o755 => Ok(()),
        Some(_) => Err(DeployError::UnsafeState),
    }
}

async fn require_kind(
    fs: &dyn RemoteFs,
    path: &str,
    expected: NodeKind,
) -> Result<(), DeployError> {
    match fs.lstat(path).await? {
        Some(node) if node.kind == expected => Ok(()),
        _ => Err(DeployError::UnsafeState),
    }
}

struct InspectedTree {
    files: BTreeMap<String, Vec<u8>>,
    directories: BTreeSet<String>,
}

#[derive(Clone, Copy)]
enum TreePolicy {
    Recognized,
    CurrentExact,
    CurrentPartial,
}

async fn inspect_tree(
    fs: &dyn RemoteFs,
    root: &str,
    policy: TreePolicy,
) -> Result<Option<InspectedTree>, DeployError> {
    match fs.lstat(root).await? {
        None => return Ok(None),
        Some(node) if node.kind == NodeKind::Directory && node.mode == 0o755 => {}
        Some(_) => return Err(DeployError::UnsafeState),
    }
    let expected_files = FILES.iter().map(|file| file.path).collect::<BTreeSet<_>>();
    let expected_dirs = expected_directories().into_iter().collect::<BTreeSet<_>>();
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    let mut stack = vec![(root.to_owned(), String::new(), 0_usize)];
    let mut entries = 0_usize;
    let mut total = 0_u64;
    while let Some((directory, relative_dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(DeployError::UnsafeState);
        }
        for name in fs.read_dir(&directory).await? {
            entries += 1;
            if entries > MAX_ENTRIES || !valid_entry_name(&name) {
                return Err(DeployError::UnsafeState);
            }
            let relative = if relative_dir.is_empty() {
                name.clone()
            } else {
                format!("{relative_dir}/{name}")
            };
            if relative.split('/').count() > MAX_DEPTH {
                return Err(DeployError::UnsafeState);
            }
            let path = join(&directory, &name);
            let node = fs.lstat(&path).await?.ok_or(DeployError::UnsafeState)?;
            match node.kind {
                NodeKind::Directory
                    if node.mode == 0o755
                        && (matches!(policy, TreePolicy::Recognized)
                            || expected_dirs.contains(relative.as_str())) =>
                {
                    if !directories.insert(relative.clone()) {
                        return Err(DeployError::UnsafeState);
                    }
                    stack.push((path, relative, depth + 1));
                }
                NodeKind::File
                    if node.mode == 0o644
                        && (matches!(policy, TreePolicy::Recognized)
                            || expected_files.contains(relative.as_str())) =>
                {
                    if node.size > MAX_FILE_SIZE {
                        return Err(DeployError::UnsafeState);
                    }
                    let bytes = fs.read(&path).await?;
                    if bytes.len() as u64 != node.size {
                        return Err(DeployError::VerificationFailed);
                    }
                    total += node.size;
                    if total > MAX_TOTAL_SIZE || files.insert(relative, bytes).is_some() {
                        return Err(DeployError::UnsafeState);
                    }
                }
                _ => return Err(DeployError::UnsafeState),
            }
        }
    }
    if matches!(policy, TreePolicy::CurrentExact)
        && (files.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_files
            || directories
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != expected_dirs)
    {
        return Err(DeployError::UnsafeState);
    }
    Ok(Some(InspectedTree { files, directories }))
}

fn valid_entry_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.contains('\\')
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn expected_directories() -> Vec<&'static str> {
    let mut directories = BTreeSet::new();
    for file in FILES {
        let mut current = file.path;
        while let Some((parent, _)) = current.rsplit_once('/') {
            directories.insert(parent);
            current = parent;
        }
    }
    directories.into_iter().collect()
}

fn installed_manifest(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<IntegrationManifest, DeployError> {
    let manifest: IntegrationManifest =
        serde_json::from_slice(files.get("manifest.json").ok_or(DeployError::UnsafeState)?)
            .map_err(|_| DeployError::UnsafeState)?;
    if manifest.domain != DOMAIN {
        return Err(DeployError::UnsafeState);
    }
    Ok(manifest)
}

fn validate_recognized_version(files: &BTreeMap<String, Vec<u8>>) -> Result<(), DeployError> {
    let manifest = installed_manifest(files)?;
    let installed = Version::parse(&manifest.version).map_err(|_| DeployError::UnsafeState)?;
    let target = Version::parse(PACKAGE_VERSION).map_err(|_| DeployError::VerificationFailed)?;
    if installed >= target {
        Err(DeployError::UnsafeState)
    } else {
        Ok(())
    }
}

fn verify_exact(tree: &InspectedTree) -> Result<(), DeployError> {
    let expected_dirs = expected_directories().into_iter().collect::<BTreeSet<_>>();
    if tree.files.len() != FILES.len()
        || tree
            .directories
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != expected_dirs
    {
        return Err(DeployError::VerificationFailed);
    }
    for expected in FILES {
        let actual = tree
            .files
            .get(expected.path)
            .ok_or(DeployError::VerificationFailed)?;
        if actual.len() != expected.bytes.len()
            || Sha256::digest(actual) != Sha256::digest(expected.bytes)
        {
            return Err(DeployError::VerificationFailed);
        }
    }
    Ok(())
}

async fn remove_package_tree(
    fs: &dyn RemoteFs,
    root: &str,
    policy: TreePolicy,
) -> Result<(), DeployError> {
    let tree = inspect_tree(fs, root, policy)
        .await?
        .ok_or(DeployError::UnsafeState)?;
    if matches!(policy, TreePolicy::Recognized) {
        validate_recognized_version(&tree.files)?;
    }
    for file in tree.files.keys() {
        fs.remove_file(&join(root, file)).await?;
    }
    let mut directories = tree.directories.into_iter().collect::<Vec<_>>();
    directories.sort_by_key(|value| std::cmp::Reverse(value.matches('/').count()));
    for directory in directories {
        fs.remove_dir(&join(root, &directory)).await?;
    }
    fs.remove_dir(root).await
}

struct NativeConnector {
    host: String,
    port: u16,
    username: String,
    password: String,
    host_key: PublicKey,
}

impl NativeConnector {
    fn new(config: &HomeAssistantSshConfig) -> Result<Self, String> {
        let password = std::fs::read_to_string(&config.password_file)
            .map_err(|_| "failed to read Home Assistant SSH password file".to_owned())?;
        if password.is_empty() || password.chars().any(char::is_control) {
            return Err("invalid Home Assistant SSH password file".to_owned());
        }
        let key_text = std::fs::read_to_string(&config.host_public_key_file)
            .map_err(|_| "failed to read Home Assistant SSH host public key file".to_owned())?;
        let mut fields = key_text.split_ascii_whitespace();
        if fields.next() != Some("ssh-ed25519") {
            return Err("invalid Home Assistant SSH host public key file".to_owned());
        }
        let encoded = fields
            .next()
            .ok_or_else(|| "invalid Home Assistant SSH host public key file".to_owned())?;
        let host_key = russh::keys::parse_public_key_base64(encoded)
            .map_err(|_| "invalid Home Assistant SSH host public key file".to_owned())?;
        if host_key.algorithm() != russh::keys::Algorithm::Ed25519
            || (fields.next().is_some() && fields.count() > 0)
        {
            return Err("invalid Home Assistant SSH host public key file".to_owned());
        }
        Ok(Self {
            host: config.host.clone(),
            port: config.port,
            username: config.username.clone(),
            password,
            host_key,
        })
    }
}

impl RemoteConnector for NativeConnector {
    fn connect(&self) -> RemoteFuture<'_, Arc<dyn RemoteFs>> {
        Box::pin(async move {
            let handler = HostKeyVerifier {
                expected: self.host_key.clone(),
            };
            let ssh_config = client::Config {
                inactivity_timeout: Some(Duration::from_secs(15)),
                ..Default::default()
            };
            let mut ssh = timeout(
                CONNECT_TIMEOUT,
                client::connect(
                    Arc::new(ssh_config),
                    (self.host.as_str(), self.port),
                    handler,
                ),
            )
            .await
            .map_err(|_| DeployError::Timeout)?
            .map_err(map_ssh_connect_error)?;
            let authenticated = timeout(
                CONNECT_TIMEOUT,
                ssh.authenticate_password(&self.username, &self.password),
            )
            .await
            .map_err(|_| DeployError::Timeout)?
            .map_err(|_| DeployError::Unavailable)?;
            if !authenticated.success() {
                return Err(DeployError::AuthenticationFailed);
            }
            let channel = timeout(CONNECT_TIMEOUT, ssh.channel_open_session())
                .await
                .map_err(|_| DeployError::Timeout)?
                .map_err(|_| DeployError::Unavailable)?;
            timeout(CONNECT_TIMEOUT, channel.request_subsystem(true, "sftp"))
                .await
                .map_err(|_| DeployError::Timeout)?
                .map_err(|_| DeployError::Unavailable)?;
            let sftp_config = russh_sftp::client::Config {
                max_packet_len: MAX_SFTP_PACKET as u32,
                max_concurrent_writes: 1,
                request_timeout_secs: 10,
            };
            let sftp = RawSftpSession::new_with_config(
                BoundedSftpStream::new(channel.into_stream()),
                sftp_config,
            );
            timeout(CONNECT_TIMEOUT, sftp.init())
                .await
                .map_err(|_| DeployError::Timeout)?
                .map_err(map_sftp_error)?;
            Ok(Arc::new(NativeFs { sftp, _ssh: ssh }) as Arc<dyn RemoteFs>)
        })
    }
}

struct HostKeyVerifier {
    expected: PublicKey,
}

impl client::Handler for HostKeyVerifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(server_public_key == &self.expected)
    }
}

fn map_ssh_connect_error(error: russh::Error) -> DeployError {
    if matches!(error, russh::Error::UnknownKey) {
        DeployError::HostKeyMismatch
    } else {
        DeployError::Unavailable
    }
}

struct NativeFs {
    sftp: RawSftpSession,
    _ssh: client::Handle<HostKeyVerifier>,
}

struct BoundedSftpStream<S> {
    inner: S,
    prefix: [u8; 4],
    prefix_len: usize,
    prefix_sent: usize,
    remaining: usize,
}

impl<S> BoundedSftpStream<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            prefix: [0; 4],
            prefix_len: 0,
            prefix_sent: 0,
            remaining: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for BoundedSftpStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.prefix_len < 4 {
                let mut prefix = ReadBuf::new(&mut this.prefix[this.prefix_len..]);
                match Pin::new(&mut this.inner).poll_read(context, &mut prefix) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(())) if prefix.filled().is_empty() => {
                        return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
                    }
                    Poll::Ready(Ok(())) => this.prefix_len += prefix.filled().len(),
                }
                if this.prefix_len < 4 {
                    continue;
                }
                this.remaining = u32::from_be_bytes(this.prefix) as usize;
                if this.remaining > MAX_SFTP_PACKET {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "SFTP packet exceeds configured limit",
                    )));
                }
            }

            if this.prefix_sent < 4 {
                let count = output.remaining().min(4 - this.prefix_sent);
                output.put_slice(&this.prefix[this.prefix_sent..this.prefix_sent + count]);
                this.prefix_sent += count;
                if output.remaining() == 0 {
                    return Poll::Ready(Ok(()));
                }
            }

            if this.remaining == 0 {
                this.prefix_len = 0;
                this.prefix_sent = 0;
                continue;
            }

            let mut buffer = [0_u8; 8192];
            let count = output.remaining().min(this.remaining).min(buffer.len());
            let mut packet = ReadBuf::new(&mut buffer[..count]);
            match Pin::new(&mut this.inner).poll_read(context, &mut packet) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) if packet.filled().is_empty() => {
                    return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
                }
                Poll::Ready(Ok(())) => {
                    output.put_slice(packet.filled());
                    this.remaining -= packet.filled().len();
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BoundedSftpStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

impl RemoteFs for NativeFs {
    fn lstat<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Option<Node>> {
        Box::pin(async move {
            match self.sftp.lstat(path).await {
                Ok(metadata) => metadata_node(metadata.attrs).map(Some),
                Err(error) if is_missing(&error) => Ok(None),
                Err(error) => Err(map_sftp_error(error)),
            }
        })
    }

    fn read_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<String>> {
        Box::pin(async move {
            let handle = self
                .sftp
                .opendir(path)
                .await
                .map_err(map_sftp_error)?
                .handle;
            let result = async {
                let mut names = Vec::new();
                loop {
                    match self.sftp.readdir(handle.clone()).await {
                        Ok(entries) => {
                            for entry in entries.files {
                                if matches!(entry.filename.as_str(), "." | "..") {
                                    continue;
                                }
                                if names.len() == MAX_ENTRIES {
                                    return Err(DeployError::UnsafeState);
                                }
                                names.push(entry.filename);
                            }
                        }
                        Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                            break;
                        }
                        Err(error) => return Err(map_sftp_error(error)),
                    }
                }
                Ok(names)
            }
            .await;
            let close = self.sftp.close(handle).await.map_err(map_sftp_error);
            match (result, close) {
                (Ok(names), Ok(_)) => Ok(names),
                (Err(error), _) | (_, Err(error)) => Err(error),
            }
        })
    }

    fn read<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let handle = self
                .sftp
                .open(path, OpenFlags::READ, FileAttributes::empty())
                .await
                .map_err(map_sftp_error)?
                .handle;
            let result = async {
                let mut bytes = Vec::new();
                loop {
                    match self
                        .sftp
                        .read(handle.clone(), bytes.len() as u64, SFTP_CHUNK)
                        .await
                    {
                        Ok(data) => {
                            if data.data.is_empty()
                                || bytes.len().saturating_add(data.data.len())
                                    > MAX_FILE_SIZE as usize
                            {
                                return if data.data.is_empty() {
                                    Ok(bytes)
                                } else {
                                    Err(DeployError::UnsafeState)
                                };
                            }
                            bytes.extend_from_slice(&data.data);
                        }
                        Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                            return Ok(bytes);
                        }
                        Err(error) => return Err(map_sftp_error(error)),
                    }
                }
            }
            .await;
            let close = self.sftp.close(handle).await.map_err(map_sftp_error);
            match (result, close) {
                (Ok(bytes), Ok(_)) => Ok(bytes),
                (Err(error), _) | (_, Err(error)) => Err(error),
            }
        })
    }

    fn mkdir<'a>(&'a self, path: &'a str, mode: u32) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.sftp
                .mkdir(
                    path,
                    FileAttributes {
                        permissions: Some(mode),
                        ..Default::default()
                    },
                )
                .await
                .map_err(map_sftp_error)?;
            self.sftp
                .setstat(
                    path,
                    FileAttributes {
                        permissions: Some(mode),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
                .map_err(map_sftp_error)
        })
    }

    fn write_exclusive<'a>(
        &'a self,
        path: &'a str,
        bytes: &'a [u8],
        mode: u32,
    ) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            let handle = self
                .sftp
                .open(
                    path,
                    OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                    FileAttributes {
                        permissions: Some(mode),
                        ..Default::default()
                    },
                )
                .await
                .map_err(map_exclusive_error)?
                .handle;
            let result = async {
                for (index, chunk) in bytes.chunks(SFTP_CHUNK as usize).enumerate() {
                    self.sftp
                        .write(
                            handle.clone(),
                            (index * SFTP_CHUNK as usize) as u64,
                            chunk.to_vec(),
                        )
                        .await
                        .map_err(map_sftp_error)?;
                }
                Ok(())
            }
            .await;
            let close = self.sftp.close(handle).await.map_err(map_sftp_error);
            result.and(close.map(|_| ()))
        })
    }

    fn rename<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.sftp
                .rename(from, to)
                .await
                .map(|_| ())
                .map_err(map_sftp_error)
        })
    }

    fn remove_file<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.sftp
                .remove(path)
                .await
                .map(|_| ())
                .map_err(map_sftp_error)
        })
    }

    fn remove_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()> {
        Box::pin(async move {
            self.sftp
                .rmdir(path)
                .await
                .map(|_| ())
                .map_err(map_sftp_error)
        })
    }
}

fn metadata_node(metadata: FileAttributes) -> Result<Node, DeployError> {
    let kind = if metadata.is_regular() {
        NodeKind::File
    } else if metadata.is_dir() {
        NodeKind::Directory
    } else {
        return Err(DeployError::UnsafeState);
    };
    Ok(Node {
        kind,
        size: metadata.len(),
        mode: metadata.permissions.ok_or(DeployError::UnsafeState)? & 0o777,
    })
}

fn is_missing(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(status) if status.status_code == StatusCode::NoSuchFile)
}

fn map_exclusive_error(error: SftpError) -> DeployError {
    if matches!(error, SftpError::Status(_)) {
        DeployError::UnsafeState
    } else {
        map_sftp_error(error)
    }
}

fn map_sftp_error(error: SftpError) -> DeployError {
    match error {
        SftpError::Timeout => DeployError::Timeout,
        SftpError::Status(status) if status.status_code == StatusCode::PermissionDenied => {
            DeployError::AuthenticationFailed
        }
        SftpError::Status(_) => DeployError::UnsafeState,
        _ => DeployError::Unavailable,
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeployInput {
    /// Must be exactly true.
    #[schemars(schema_with = "true_schema")]
    pub confirm: bool,
}

impl DeployInput {
    pub fn validate(self) -> Result<(), DeployError> {
        self.confirm
            .then_some(())
            .ok_or(DeployError::InvalidArguments)
    }
}

fn true_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::from_value(serde_json::json!({"type":"boolean","const":true}))
        .expect("valid schema")
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Clone)]
    enum FakeEntry {
        File { bytes: Vec<u8>, mode: u32 },
        Directory { mode: u32 },
        Symlink,
    }

    #[derive(Default)]
    struct FakeFs {
        entries: Mutex<BTreeMap<String, FakeEntry>>,
        fail_rename: Mutex<Vec<(String, String)>>,
        fail_remove: Mutex<Vec<String>>,
        fail_write: Mutex<Vec<String>>,
        mutations: Mutex<Vec<String>>,
        corrupt_upload: AtomicBool,
        pause_rename: Mutex<Option<RenamePause>>,
        pause_write: Mutex<Option<(String, Arc<tokio::sync::Barrier>)>>,
    }

    type RenamePause = (
        String,
        String,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
    );

    impl FakeFs {
        fn base() -> Arc<Self> {
            let fs = Arc::new(Self::default());
            fs.entries
                .lock()
                .unwrap()
                .insert("/config".into(), FakeEntry::Directory { mode: 0o750 });
            fs
        }

        fn add_package(&self, root: &str, version: &str, drift: bool) {
            let mut entries = self.entries.lock().unwrap();
            entries.insert(root.into(), FakeEntry::Directory { mode: 0o755 });
            for directory in expected_directories() {
                entries.insert(join(root, directory), FakeEntry::Directory { mode: 0o755 });
            }
            for file in FILES {
                let mut bytes = file.bytes.to_vec();
                if file.path == "manifest.json" && version != PACKAGE_VERSION {
                    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    value["version"] = version.into();
                    bytes = serde_json::to_vec(&value).unwrap();
                } else if drift && file.path == "const.py" {
                    bytes.push(b' ');
                }
                entries.insert(
                    join(root, file.path),
                    FakeEntry::File { bytes, mode: 0o644 },
                );
            }
        }

        fn add_historical_package(&self, root: &str, version: &str, marker: &str) {
            self.add_package(root, version, false);
            let mut entries = self.entries.lock().unwrap();
            entries.remove(&join(root, "websocket_api.py"));
            entries.insert(join(root, "legacy"), FakeEntry::Directory { mode: 0o755 });
            entries.insert(
                join(root, &format!("legacy/{marker}.py")),
                FakeEntry::File {
                    bytes: b"historical".to_vec(),
                    mode: 0o644,
                },
            );
        }

        fn has(&self, path: &str) -> bool {
            self.entries.lock().unwrap().contains_key(path)
        }

        fn add_lock_file(&self, path: &str, lease_at: u64, owner: u8) {
            let bytes = serde_json::to_vec(&LockFile {
                schema: 1,
                owner: format!("{owner:02x}").repeat(OWNER_TOKEN_BYTES),
                lease_at,
            })
            .unwrap();
            self.entries
                .lock()
                .unwrap()
                .insert(path.into(), FakeEntry::File { bytes, mode: 0o600 });
        }

        fn add_journal(&self, operation: Operation) {
            let bytes = serde_json::to_vec(&Journal {
                schema: 1,
                operation,
                target_version: PACKAGE_VERSION.into(),
            })
            .unwrap();
            self.entries.lock().unwrap().insert(
                "/config/.smarthome_mcp-deploy/journal.json".into(),
                FakeEntry::File { bytes, mode: 0o600 },
            );
        }
    }

    impl RemoteFs for FakeFs {
        fn lstat<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Option<Node>> {
            Box::pin(async move {
                Ok(self
                    .entries
                    .lock()
                    .unwrap()
                    .get(path)
                    .map(|entry| match entry {
                        FakeEntry::File { bytes, mode } => Node {
                            kind: NodeKind::File,
                            size: bytes.len() as u64,
                            mode: *mode,
                        },
                        FakeEntry::Directory { mode } => Node {
                            kind: NodeKind::Directory,
                            size: 0,
                            mode: *mode,
                        },
                        FakeEntry::Symlink => Node {
                            kind: NodeKind::File,
                            size: u64::MAX,
                            mode: 0,
                        },
                    }))
            })
        }

        fn read_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<String>> {
            Box::pin(async move {
                let prefix = format!("{}/", path.trim_end_matches('/'));
                let mut names = BTreeSet::new();
                for key in self.entries.lock().unwrap().keys() {
                    if let Some(rest) = key.strip_prefix(&prefix)
                        && let Some(name) = rest.split('/').next().filter(|name| !name.is_empty())
                    {
                        names.insert(name.to_owned());
                    }
                }
                Ok(names.into_iter().collect())
            })
        }

        fn read<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, Vec<u8>> {
            Box::pin(async move {
                match self.entries.lock().unwrap().get(path) {
                    Some(FakeEntry::File { bytes, .. }) => Ok(bytes.clone()),
                    _ => Err(DeployError::UnsafeState),
                }
            })
        }

        fn mkdir<'a>(&'a self, path: &'a str, mode: u32) -> RemoteFuture<'a, ()> {
            Box::pin(async move {
                let mut entries = self.entries.lock().unwrap();
                if entries
                    .insert(path.into(), FakeEntry::Directory { mode })
                    .is_some()
                {
                    return Err(DeployError::UnsafeState);
                }
                self.mutations.lock().unwrap().push(format!("mkdir:{path}"));
                Ok(())
            })
        }

        fn write_exclusive<'a>(
            &'a self,
            path: &'a str,
            bytes: &'a [u8],
            mode: u32,
        ) -> RemoteFuture<'a, ()> {
            Box::pin(async move {
                if self
                    .fail_write
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|value| value == path)
                {
                    return Err(DeployError::Unavailable);
                }
                let barrier =
                    self.pause_write
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|(pause_path, barrier)| {
                            (pause_path == path).then(|| barrier.clone())
                        });
                if let Some(barrier) = barrier {
                    barrier.wait().await;
                }
                let mut entries = self.entries.lock().unwrap();
                if entries.contains_key(path) {
                    return Err(DeployError::UnsafeState);
                }
                let mut bytes = bytes.to_vec();
                if self.corrupt_upload.load(Ordering::Relaxed) && path.ends_with("/const.py") {
                    bytes.push(b' ');
                }
                entries.insert(path.into(), FakeEntry::File { bytes, mode });
                self.mutations.lock().unwrap().push(format!("write:{path}"));
                Ok(())
            })
        }

        fn rename<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteFuture<'a, ()> {
            Box::pin(async move {
                if self
                    .fail_rename
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|pair| pair.0 == from && pair.1 == to)
                {
                    return Err(DeployError::Unavailable);
                }
                {
                    let pause = self.pause_rename.lock().unwrap().as_ref().and_then(
                        |(pause_from, pause_to, reached, release)| {
                            (pause_from == from && pause_to == to)
                                .then(|| (reached.clone(), release.clone()))
                        },
                    );
                    if let Some((reached, release)) = pause {
                        reached.notify_one();
                        release.notified().await;
                    }
                }
                {
                    let mut entries = self.entries.lock().unwrap();
                    if entries.contains_key(to) || !entries.contains_key(from) {
                        return Err(DeployError::UnsafeState);
                    }
                    let moving = entries
                        .keys()
                        .filter(|path| *path == from || path.starts_with(&format!("{from}/")))
                        .cloned()
                        .collect::<Vec<_>>();
                    for old in moving {
                        let entry = entries.remove(&old).unwrap();
                        entries.insert(format!("{to}{}", &old[from.len()..]), entry);
                    }
                }
                self.mutations
                    .lock()
                    .unwrap()
                    .push(format!("rename:{from}:{to}"));
                Ok(())
            })
        }

        fn remove_file<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()> {
            Box::pin(async move {
                if self
                    .fail_remove
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|value| value == path)
                {
                    return Err(DeployError::Unavailable);
                }
                match self.entries.lock().unwrap().remove(path) {
                    Some(FakeEntry::File { .. }) => {
                        self.mutations
                            .lock()
                            .unwrap()
                            .push(format!("remove:{path}"));
                        Ok(())
                    }
                    _ => Err(DeployError::UnsafeState),
                }
            })
        }

        fn remove_dir<'a>(&'a self, path: &'a str) -> RemoteFuture<'a, ()> {
            Box::pin(async move {
                let mut entries = self.entries.lock().unwrap();
                if entries
                    .keys()
                    .any(|key| key.starts_with(&format!("{path}/")))
                {
                    return Err(DeployError::UnsafeState);
                }
                match entries.remove(path) {
                    Some(FakeEntry::Directory { .. }) => {
                        self.mutations.lock().unwrap().push(format!("rmdir:{path}"));
                        Ok(())
                    }
                    _ => Err(DeployError::UnsafeState),
                }
            })
        }
    }

    struct FakeConnector(Arc<FakeFs>);

    impl RemoteConnector for FakeConnector {
        fn connect(&self) -> RemoteFuture<'_, Arc<dyn RemoteFs>> {
            let fs = self.0.clone();
            Box::pin(async move { Ok(fs as Arc<dyn RemoteFs>) })
        }
    }

    fn deployer(fs: Arc<FakeFs>) -> ComponentDeployer {
        ComponentDeployer::new(Arc::new(FakeConnector(fs)))
    }

    #[test]
    fn embedded_files_match_the_installable_repository_tree() {
        validate_embedded().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("custom_components/smarthome_mcp");
        let mut actual = BTreeMap::new();
        fn visit(root: &Path, path: &Path, actual: &mut BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let file_type = entry.file_type().unwrap();
                assert!(!file_type.is_symlink());
                let child = entry.path();
                let relative = child.strip_prefix(root).unwrap();
                if file_type.is_dir() {
                    if entry.file_name() != "__pycache__" {
                        visit(root, &child, actual);
                    }
                } else if !matches!(
                    child.extension().and_then(|value| value.to_str()),
                    Some("pyc" | "pyo")
                ) {
                    let relative = relative.to_str().unwrap().replace('\\', "/");
                    assert!(valid_relative_path(&relative));
                    actual.insert(relative, std::fs::read(child).unwrap());
                }
            }
        }
        visit(&source, &source, &mut actual);
        let expected = FILES
            .iter()
            .map(|file| (file.path.to_owned(), file.bytes.to_vec()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn deploy_input_is_closed_and_requires_true() {
        let schema = serde_json::to_value(schemars::schema_for!(DeployInput)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["confirm"]["const"], true);
        assert!(DeployInput { confirm: false }.validate().is_err());
        assert!(DeployInput { confirm: true }.validate().is_ok());
    }

    #[tokio::test]
    async fn installs_updates_and_noops_with_bounded_output() {
        let fs = FakeFs::base();
        let result = deployer(fs.clone()).deploy().await.unwrap();
        assert_eq!(result, output("install", true, None));
        assert!(fs.has("/config/custom_components/smarthome_mcp/manifest.json"));

        let result = deployer(fs.clone()).deploy().await.unwrap();
        assert_eq!(result, output("noop", false, None));

        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package("/config/custom_components/smarthome_mcp", "0.1.0", false);
        let result = deployer(fs.clone()).deploy().await.unwrap();
        assert_eq!(result, output("update", true, Some("0.1.0".into())));
        assert!(fs.has("/config/.smarthome_mcp-deploy/backup/manifest.json"));
    }

    #[tokio::test]
    async fn historical_active_path_sets_update_successfully() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_historical_package(
            "/config/custom_components/smarthome_mcp",
            "0.1.0",
            "retired_active",
        );

        let result = deployer(fs.clone()).deploy().await.unwrap();

        assert_eq!(result, output("update", true, Some("0.1.0".into())));
        assert!(fs.has("/config/custom_components/smarthome_mcp/websocket_api.py"));
        assert!(!fs.has("/config/custom_components/smarthome_mcp/legacy"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/backup/legacy/retired_active.py"));
    }

    #[tokio::test]
    async fn historical_retained_backup_is_removed_before_the_next_update() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_historical_package(
            "/config/custom_components/smarthome_mcp",
            "0.1.1",
            "next_backup",
        );
        fs.add_historical_package(
            "/config/.smarthome_mcp-deploy/backup",
            "0.1.0",
            "retained_backup",
        );

        let result = deployer(fs.clone()).deploy().await.unwrap();

        assert_eq!(result, output("update", true, Some("0.1.1".into())));
        assert!(!fs.has("/config/.smarthome_mcp-deploy/backup/legacy/retained_backup.py"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/backup/legacy/next_backup.py"));
    }

    #[tokio::test]
    async fn rejects_equal_version_drift_downgrade_and_unsafe_entries() {
        for (version, drift) in [(PACKAGE_VERSION, true), ("99.0.0", false)] {
            let fs = FakeFs::base();
            fs.entries.lock().unwrap().insert(
                "/config/custom_components".into(),
                FakeEntry::Directory { mode: 0o755 },
            );
            fs.add_package("/config/custom_components/smarthome_mcp", version, drift);
            assert_eq!(
                deployer(fs).deploy().await.unwrap_err(),
                DeployError::UnsafeState
            );
        }
        let fs = FakeFs::base();
        fs.entries
            .lock()
            .unwrap()
            .insert("/config/custom_components".into(), FakeEntry::Symlink);
        assert_eq!(
            deployer(fs).deploy().await.unwrap_err(),
            DeployError::UnsafeState
        );

        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package(
            "/config/custom_components/smarthome_mcp",
            PACKAGE_VERSION,
            false,
        );
        fs.entries.lock().unwrap().insert(
            "/config/custom_components/smarthome_mcp/unexpected.py".into(),
            FakeEntry::File {
                bytes: vec![0; MAX_FILE_SIZE as usize + 1],
                mode: 0o644,
            },
        );
        assert_eq!(
            deployer(fs).deploy().await.unwrap_err(),
            DeployError::UnsafeState
        );

        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package(
            "/config/custom_components/smarthome_mcp",
            PACKAGE_VERSION,
            false,
        );
        fs.entries.lock().unwrap().insert(
            "/config/custom_components/smarthome_mcp/extra_empty".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        assert_eq!(
            deployer(fs).deploy().await.unwrap_err(),
            DeployError::UnsafeState
        );
    }

    #[tokio::test]
    async fn failed_second_rename_restores_the_previous_active_tree() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package("/config/custom_components/smarthome_mcp", "0.1.0", false);
        *fs.fail_rename.lock().unwrap() = vec![(
            "/config/.smarthome_mcp-deploy/staging".into(),
            "/config/custom_components/smarthome_mcp".into(),
        )];
        assert_eq!(
            deployer(fs.clone()).deploy().await.unwrap_err(),
            DeployError::Unavailable
        );
        let tree = inspect_tree(
            fs.as_ref(),
            "/config/custom_components/smarthome_mcp",
            TreePolicy::Recognized,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(installed_manifest(&tree.files).unwrap().version, "0.1.0");
    }

    #[tokio::test]
    async fn staged_readback_detects_corrupted_uploads_before_commit() {
        let fs = FakeFs::base();
        fs.corrupt_upload.store(true, Ordering::Relaxed);
        assert_eq!(
            deployer(fs.clone()).deploy().await.unwrap_err(),
            DeployError::VerificationFailed
        );
        assert!(!fs.has("/config/custom_components/smarthome_mcp"));
        assert!(!fs.has("/config/.smarthome_mcp-deploy/staging"));
    }

    #[tokio::test]
    async fn rollback_failure_is_distinct_and_preserves_transaction_state() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package("/config/custom_components/smarthome_mcp", "0.1.0", false);
        *fs.fail_rename.lock().unwrap() = vec![
            (
                "/config/.smarthome_mcp-deploy/staging".into(),
                "/config/custom_components/smarthome_mcp".into(),
            ),
            (
                "/config/.smarthome_mcp-deploy/backup".into(),
                "/config/custom_components/smarthome_mcp".into(),
            ),
        ];
        assert_eq!(
            deployer(fs.clone()).deploy().await.unwrap_err(),
            DeployError::RollbackFailed
        );
        assert!(fs.has("/config/.smarthome_mcp-deploy/backup/manifest.json"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/staging/manifest.json"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/journal.json"));
    }

    #[tokio::test]
    async fn process_local_capacity_is_non_waiting() {
        struct PendingConnector;
        impl RemoteConnector for PendingConnector {
            fn connect(&self) -> RemoteFuture<'_, Arc<dyn RemoteFs>> {
                Box::pin(std::future::pending())
            }
        }
        let deployer = ComponentDeployer::new(Arc::new(PendingConnector));
        let first = tokio::spawn({
            let deployer = deployer.clone();
            async move { deployer.deploy().await }
        });
        tokio::task::yield_now().await;
        assert_eq!(
            deployer.deploy().await.unwrap_err(),
            DeployError::CapacityExhausted
        );
        first.abort();
    }

    #[tokio::test]
    async fn dropping_the_request_after_the_first_rename_does_not_abandon_commit() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package("/config/custom_components/smarthome_mcp", "0.1.0", false);
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *fs.pause_rename.lock().unwrap() = Some((
            "/config/custom_components/smarthome_mcp".into(),
            "/config/.smarthome_mcp-deploy/backup".into(),
            reached.clone(),
            release.clone(),
        ));
        let task = tokio::spawn({
            let deployer = deployer(fs.clone());
            async move { deployer.deploy().await }
        });
        reached.notified().await;
        task.abort();
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !fs.has("/config/.smarthome_mcp-deploy/lock")
                    && fs.has("/config/custom_components/smarthome_mcp/manifest.json")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let tree = inspect_tree(
            fs.as_ref(),
            "/config/custom_components/smarthome_mcp",
            TreePolicy::Recognized,
        )
        .await
        .unwrap()
        .unwrap();
        verify_exact(&tree).unwrap();
    }

    #[tokio::test]
    async fn fresh_remote_lock_fails_without_waiting() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        let lock = serde_json::to_vec(&LockFile {
            schema: 1,
            owner: "a".repeat(OWNER_TOKEN_BYTES * 2),
            lease_at: unix_time().unwrap(),
        })
        .unwrap();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy/lock".into(),
            FakeEntry::File {
                bytes: lock,
                mode: 0o600,
            },
        );
        assert_eq!(
            deployer(fs).deploy().await.unwrap_err(),
            DeployError::CapacityExhausted
        );
    }

    #[tokio::test]
    async fn concurrent_stale_claim_recovery_yields_one_owner() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_lock_file(
            "/config/.smarthome_mcp-deploy/lock.claim",
            unix_time().unwrap() - LOCK_STALE_AFTER_SECS - 1,
            1,
        );
        *fs.pause_write.lock().unwrap() = Some((
            "/config/.smarthome_mcp-deploy/lock".into(),
            Arc::new(tokio::sync::Barrier::new(2)),
        ));
        let (results, mut received) = tokio::sync::mpsc::channel(2);
        let release = Arc::new(tokio::sync::Notify::new());
        for _ in 0..2 {
            let fs = fs.clone();
            let results = results.clone();
            let release = release.clone();
            tokio::spawn(async move {
                let paths = Paths::new("/config");
                let result = match acquire_lock(fs.as_ref(), &paths).await {
                    Ok(owned) => {
                        release.notified().await;
                        owned.unlock().await
                    }
                    Err(error) => Err(error),
                };
                results.send(result).await.unwrap();
            });
        }
        drop(results);

        assert_eq!(
            received.recv().await.unwrap(),
            Err(DeployError::CapacityExhausted)
        );
        release.notify_one();
        assert_eq!(received.recv().await.unwrap(), Ok(()));
        assert!(!fs.has("/config/.smarthome_mcp-deploy/lock"));
        assert!(!fs.has("/config/.smarthome_mcp-deploy/lock.claim"));
    }

    #[tokio::test]
    async fn stale_observer_does_not_remove_a_replacement_owner() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        let lock_path = "/config/.smarthome_mcp-deploy/lock";
        let claim_path = "/config/.smarthome_mcp-deploy/lock.claim";
        fs.add_lock_file(
            lock_path,
            unix_time().unwrap() - LOCK_STALE_AFTER_SECS - 1,
            1,
        );
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *fs.pause_rename.lock().unwrap() = Some((
            lock_path.into(),
            claim_path.into(),
            reached.clone(),
            release.clone(),
        ));
        let task = tokio::spawn({
            let fs = fs.clone();
            async move {
                let paths = Paths::new("/config");
                acquire_lock(fs.as_ref(), &paths).await.map(|_| ())
            }
        });
        reached.notified().await;
        fs.entries.lock().unwrap().remove(lock_path);
        fs.add_lock_file(lock_path, unix_time().unwrap(), 2);
        let replacement = match fs.entries.lock().unwrap().get(lock_path).unwrap() {
            FakeEntry::File { bytes, .. } => bytes.clone(),
            _ => unreachable!(),
        };
        release.notify_one();

        assert_eq!(task.await.unwrap(), Err(DeployError::UnsafeState));
        assert!(!fs.has(lock_path));
        assert_eq!(
            match fs.entries.lock().unwrap().get(claim_path).unwrap() {
                FakeEntry::File { bytes, .. } => bytes.clone(),
                _ => unreachable!(),
            },
            replacement
        );
    }

    #[tokio::test]
    async fn mutation_and_unlock_require_the_exact_owner() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        let paths = Paths::new("/config");
        let owned = acquire_lock(fs.as_ref(), &paths).await.unwrap();
        fs.add_lock_file(&paths.lock, unix_time().unwrap(), 9);

        assert_eq!(
            owned.mkdir("/config/should-not-exist", 0o755).await,
            Err(DeployError::UnsafeState)
        );
        assert_eq!(owned.unlock().await, Err(DeployError::UnsafeState));
        assert!(!fs.has("/config/should-not-exist"));
        assert!(fs.has(&paths.lock));
    }

    #[tokio::test]
    async fn abandoned_stale_claim_is_recovered() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_lock_file(
            "/config/.smarthome_mcp-deploy/lock.claim",
            unix_time().unwrap() - LOCK_STALE_AFTER_SECS - 1,
            1,
        );

        assert_eq!(
            deployer(fs.clone()).deploy().await.unwrap(),
            output("install", true, None)
        );
        assert!(!fs.has("/config/.smarthome_mcp-deploy/lock.claim"));
    }

    #[tokio::test]
    async fn managed_directories_and_private_files_require_exact_modes() {
        for path in ["/config/custom_components", "/config/.smarthome_mcp-deploy"] {
            let fs = FakeFs::base();
            fs.entries
                .lock()
                .unwrap()
                .insert(path.into(), FakeEntry::Directory { mode: 0o700 });
            assert_eq!(
                deployer(fs).deploy().await.unwrap_err(),
                DeployError::UnsafeState
            );
        }

        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package(
            "/config/custom_components/smarthome_mcp",
            PACKAGE_VERSION,
            false,
        );
        if let Some(FakeEntry::Directory { mode }) = fs
            .entries
            .lock()
            .unwrap()
            .get_mut("/config/custom_components/smarthome_mcp")
        {
            *mode = 0o700;
        }
        assert_eq!(
            deployer(fs).deploy().await.unwrap_err(),
            DeployError::UnsafeState
        );

        for private_path in [
            "/config/.smarthome_mcp-deploy/lock",
            "/config/.smarthome_mcp-deploy/lock.claim",
        ] {
            let fs = FakeFs::base();
            fs.entries.lock().unwrap().insert(
                "/config/.smarthome_mcp-deploy".into(),
                FakeEntry::Directory { mode: 0o755 },
            );
            fs.add_lock_file(private_path, unix_time().unwrap(), 1);
            if let Some(FakeEntry::File { mode, .. }) =
                fs.entries.lock().unwrap().get_mut(private_path)
            {
                *mode = 0o644;
            }
            assert_eq!(
                deployer(fs).deploy().await.unwrap_err(),
                DeployError::UnsafeState
            );
        }

        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_journal(Operation::Install);
        if let Some(FakeEntry::File { mode, .. }) = fs
            .entries
            .lock()
            .unwrap()
            .get_mut("/config/.smarthome_mcp-deploy/journal.json")
        {
            *mode = 0o644;
        }
        assert_eq!(
            deployer(fs).deploy().await.unwrap_err(),
            DeployError::UnsafeState
        );
    }

    #[tokio::test]
    async fn invalid_backups_are_rejected_before_recovery_mutation() {
        enum InvalidBackup {
            Symlink,
            Malformed,
            Equal,
            Newer,
        }
        for invalid in [
            InvalidBackup::Symlink,
            InvalidBackup::Malformed,
            InvalidBackup::Equal,
            InvalidBackup::Newer,
        ] {
            let fs = FakeFs::base();
            fs.entries.lock().unwrap().insert(
                "/config/custom_components".into(),
                FakeEntry::Directory { mode: 0o755 },
            );
            fs.entries.lock().unwrap().insert(
                "/config/.smarthome_mcp-deploy".into(),
                FakeEntry::Directory { mode: 0o755 },
            );
            fs.add_package(
                "/config/.smarthome_mcp-deploy/staging",
                PACKAGE_VERSION,
                false,
            );
            match invalid {
                InvalidBackup::Symlink => {
                    fs.entries.lock().unwrap().insert(
                        "/config/.smarthome_mcp-deploy/backup".into(),
                        FakeEntry::Symlink,
                    );
                }
                InvalidBackup::Malformed => {
                    fs.add_package("/config/.smarthome_mcp-deploy/backup", "0.1.0", false);
                    fs.entries.lock().unwrap().insert(
                        "/config/.smarthome_mcp-deploy/backup/manifest.json".into(),
                        FakeEntry::File {
                            bytes: b"{".to_vec(),
                            mode: 0o644,
                        },
                    );
                }
                InvalidBackup::Equal => fs.add_package(
                    "/config/.smarthome_mcp-deploy/backup",
                    PACKAGE_VERSION,
                    false,
                ),
                InvalidBackup::Newer => {
                    fs.add_package("/config/.smarthome_mcp-deploy/backup", "99.0.0", false)
                }
            }
            fs.add_journal(Operation::Update);
            fs.mutations.lock().unwrap().clear();

            assert_eq!(
                reconcile(fs.as_ref(), &Paths::new("/config")).await,
                Err(DeployError::UnsafeState)
            );
            assert!(fs.mutations.lock().unwrap().is_empty());
            assert!(fs.has("/config/.smarthome_mcp-deploy/journal.json"));
            assert!(fs.has("/config/.smarthome_mcp-deploy/staging"));
        }
    }

    #[tokio::test]
    async fn completed_update_rejects_invalid_backup_before_journal_cleanup() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package(
            "/config/custom_components/smarthome_mcp",
            PACKAGE_VERSION,
            false,
        );
        fs.add_package(
            "/config/.smarthome_mcp-deploy/backup",
            PACKAGE_VERSION,
            false,
        );
        fs.add_journal(Operation::Update);
        fs.mutations.lock().unwrap().clear();

        assert_eq!(
            reconcile(fs.as_ref(), &Paths::new("/config")).await,
            Err(DeployError::UnsafeState)
        );
        assert!(fs.mutations.lock().unwrap().is_empty());
        assert!(fs.has("/config/.smarthome_mcp-deploy/journal.json"));
    }

    #[tokio::test]
    async fn journal_and_lock_cleanup_failures_require_recovery() {
        let journal = "/config/.smarthome_mcp-deploy/journal.json";
        let lock = "/config/.smarthome_mcp-deploy/lock";

        let fs = FakeFs::base();
        *fs.fail_remove.lock().unwrap() = vec![journal.into()];
        assert_eq!(
            deployer(fs.clone()).deploy().await.unwrap_err(),
            DeployError::CleanupIncomplete
        );
        assert!(fs.has(journal));
        assert!(!fs.has(lock));
        assert!(fs.has("/config/custom_components/smarthome_mcp"));

        let fs = FakeFs::base();
        *fs.fail_remove.lock().unwrap() = vec![lock.into()];
        assert_eq!(
            deployer(fs.clone()).deploy().await.unwrap_err(),
            DeployError::CleanupIncomplete
        );
        assert!(fs.has(lock));
        assert!(fs.has("/config/custom_components/smarthome_mcp"));
    }

    #[tokio::test]
    async fn transaction_timeout_cancels_work_then_unlocks() {
        let fs = FakeFs::base();
        let reached = Arc::new(tokio::sync::Notify::new());
        *fs.pause_rename.lock().unwrap() = Some((
            "/config/.smarthome_mcp-deploy/staging".into(),
            "/config/custom_components/smarthome_mcp".into(),
            reached.clone(),
            Arc::new(tokio::sync::Notify::new()),
        ));

        assert_eq!(
            run_transaction_with_timeout(fs.as_ref(), "/config", Duration::from_millis(1)).await,
            Err(DeployError::Timeout)
        );
        assert!(!fs.has("/config/.smarthome_mcp-deploy/lock"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/journal.json"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/staging"));
    }

    #[test]
    fn transaction_timeout_precedes_lock_staleness() {
        assert!(TRANSACTION_TIMEOUT < Duration::from_secs(LOCK_STALE_AFTER_SECS));
    }

    #[tokio::test]
    async fn recognized_interrupted_update_is_recovered_before_redeployment() {
        let fs = FakeFs::base();
        fs.entries.lock().unwrap().insert(
            "/config/custom_components".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy".into(),
            FakeEntry::Directory { mode: 0o755 },
        );
        fs.add_package("/config/.smarthome_mcp-deploy/backup", "0.1.0", false);
        fs.add_package(
            "/config/.smarthome_mcp-deploy/staging",
            PACKAGE_VERSION,
            false,
        );
        let journal = serde_json::to_vec(&Journal {
            schema: 1,
            operation: Operation::Update,
            target_version: PACKAGE_VERSION.into(),
        })
        .unwrap();
        fs.entries.lock().unwrap().insert(
            "/config/.smarthome_mcp-deploy/journal.json".into(),
            FakeEntry::File {
                bytes: journal,
                mode: 0o600,
            },
        );
        let result = deployer(fs.clone()).deploy().await.unwrap();
        assert_eq!(result.operation, "update");
        assert_eq!(result.previous_version.as_deref(), Some("0.1.0"));
        assert!(fs.has("/config/.smarthome_mcp-deploy/backup/manifest.json"));
    }

    #[tokio::test]
    async fn host_key_verifier_rejects_mismatch() {
        let expected = russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ",
        )
        .unwrap();
        let presented = russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF",
        )
        .unwrap();
        let mut verifier = HostKeyVerifier { expected };
        assert!(
            !client::Handler::check_server_key(&mut verifier, &presented)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn sftp_stream_rejects_oversized_packets_before_payload_allocation() {
        let (mut writer, reader) = tokio::io::duplex(16);
        writer
            .write_all(&((MAX_SFTP_PACKET as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        let mut bounded = BoundedSftpStream::new(reader);
        let mut prefix = [0_u8; 4];
        assert_eq!(
            bounded.read_exact(&mut prefix).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn safe_errors_do_not_contain_target_or_protocol_details() {
        for error in [
            DeployError::HostKeyMismatch,
            DeployError::AuthenticationFailed,
            DeployError::UnsafeState,
            DeployError::RollbackFailed,
            DeployError::CleanupIncomplete,
        ] {
            let value = error.into_tool_error().into_mcp_result().raw;
            let text = serde_json::to_string(&value).unwrap();
            assert!(!text.contains("/config"));
            assert!(!text.contains("172.16"));
            assert!(!text.contains("root"));
        }
    }
}
