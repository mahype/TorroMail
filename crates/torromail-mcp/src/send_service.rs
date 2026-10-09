//! Durable submission and copy service shared by MCP and the app's CLI bridge.
//! Private spool files contain MIME, never credentials. SMTP is attempted only
//! from Prepared under an OS lock; every later invocation is read/reconcile only.
use crate::{DraftRecord, LineMcpServer, ToolFailure, now_secs, sha256_hex};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use torromail_core::smtp::SubmissionError;
use torromail_core::{AccountId, Capability, CoreError, DraftFailureKind, SpecialMailboxRole};

pub(crate) mod account_id_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use torromail_core::AccountId;
    pub fn serialize<S: Serializer>(id: &AccountId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(id.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(decoder: D) -> Result<AccountId, D::Error> {
        String::deserialize(decoder).map(AccountId::new)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CopyStrategy {
    #[default]
    Imap,
    Provider,
    None,
}
impl CopyStrategy {
    pub(crate) fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value {
            None => Ok(Self::Imap),
            Some(Value::String(value)) => match value.as_str() {
                "imap" => Ok(Self::Imap),
                "provider" => Ok(Self::Provider),
                "none" => Ok(Self::None),
                _ => Err("sent_copy_strategy must be imap, provider or none".into()),
            },
            _ => Err("sent_copy_strategy must be imap, provider or none".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Submission {
    Prepared,
    Submitting,
    Accepted,
    NotAccepted,
    Unknown,
    Rejected,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CopyStatus {
    Pending,
    Appending,
    Saved,
    Provider,
    Skipped,
}
#[derive(Serialize, Deserialize)]
struct Draft {
    owner: String,
    fingerprint: String,
    record: DraftRecord,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    operation_id: String,
    account_id: String,
    owner: String,
    draft_id: String,
    fingerprint: String,
    code: String,
    expires_at: u64,
    submission: Submission,
    copy: CopyStatus,
    strategy: CopyStrategy,
    mailbox: Option<String>,
    #[serde(default)]
    copy_warning: Option<String>,
}

pub(crate) struct Store {
    directory: PathBuf,
}
struct Locked {
    _file: File,
}
fn failure() -> ToolFailure {
    ToolFailure::Io(
        "cannot safely persist send state; check TorroMail's storage permissions and free space"
            .into(),
    )
}
impl Store {
    pub(crate) fn open(policy: &Path) -> Result<Self, ToolFailure> {
        let policy = fs::canonicalize(policy).map_err(|_| failure())?;
        let root = policy.parent().ok_or_else(failure)?.join("send-state");
        let directory = root.join(sha256_hex(&policy.to_string_lossy()));
        for path in [&root, &directory] {
            fs::create_dir_all(path).map_err(|_| failure())?;
            if fs::symlink_metadata(path)
                .map_err(|_| failure())?
                .file_type()
                .is_symlink()
            {
                return Err(failure());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(|_| failure())?;
            }
        }
        Ok(Self { directory })
    }
    fn path(&self, id: &str) -> Result<PathBuf, ToolFailure> {
        if !id.starts_with("send-") && !id.starts_with("draft-") {
            return Err(failure());
        }
        let (_, hash) = id.split_once('-').ok_or_else(failure)?;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(failure());
        }
        Ok(self.directory.join(format!("{id}.json")))
    }
    fn lock(&self, id: &str) -> Result<Locked, ToolFailure> {
        let path = self.path(id)?.with_extension("lock");
        let file = private_options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|_| failure())?;
        file.lock_exclusive().map_err(|_| failure())?;
        Ok(Locked { _file: file })
    }
    fn try_lock(&self, id: &str) -> Result<Option<Locked>, ToolFailure> {
        let file = private_options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.path(id)?.with_extension("lock"))
            .map_err(|_| failure())?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Locked { _file: file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(_) => Err(failure()),
        }
    }
    fn read<T: for<'de> Deserialize<'de>>(&self, id: &str) -> Result<T, ToolFailure> {
        serde_json::from_slice(&fs::read(self.path(id)?).map_err(|_| failure())?)
            .map_err(|_| failure())
    }
    fn write<T: Serialize>(&self, id: &str, value: &T) -> Result<(), ToolFailure> {
        let path = self.path(id)?;
        let temporary = path.with_extension("tmp");
        let bytes = serde_json::to_vec(value).map_err(|_| failure())?;
        let mut file = private_options()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|_| failure())?;
        file.write_all(&bytes).map_err(|_| failure())?;
        file.sync_all().map_err(|_| failure())?;
        fs::rename(temporary, path).map_err(|_| failure())?;
        #[cfg(unix)]
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| failure())?;
        Ok(())
    }
    pub(crate) fn save_draft(
        &self,
        id: &str,
        owner: &str,
        record: &DraftRecord,
    ) -> Result<(), ToolFailure> {
        let _lock = self.lock(id)?;
        let fingerprint = fingerprint(record)?;
        if self.path(id)?.exists() {
            let existing: Draft = self.read(id)?;
            if existing.owner != owner || existing.fingerprint != fingerprint {
                return Err(changed());
            }
            return Ok(());
        }
        self.write(
            id,
            &Draft {
                owner: owner.into(),
                fingerprint,
                record: record.clone(),
            },
        )
    }
    pub(crate) fn draft(&self, id: &str, owner: &str) -> Result<DraftRecord, ToolFailure> {
        let stored: Draft = self.read(id)?;
        if stored.owner != owner {
            return Err(ToolFailure::InvalidParams(
                "draft belongs to a different client".into(),
            ));
        }
        if stored.fingerprint != fingerprint(&stored.record)? {
            return Err(changed());
        }
        Ok(stored.record)
    }
    fn ids(&self) -> Result<Vec<String>, ToolFailure> {
        fs::read_dir(&self.directory)
            .map_err(|_| failure())?
            .map(|entry| {
                let path = entry.map_err(|_| failure())?.path();
                Ok(path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .filter(|name| {
                        name.starts_with("send-")
                            && path.extension().is_some_and(|ext| ext == "json")
                    })
                    .map(str::to_owned))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|ids| ids.into_iter().flatten().collect())
    }
}
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn fingerprint(record: &DraftRecord) -> Result<String, ToolFailure> {
    serde_json::to_string(record)
        .map(|text| sha256_hex(&text))
        .map_err(|_| failure())
}
fn changed() -> ToolFailure {
    ToolFailure::InvalidParams(
        "message contents changed after approval; create a new draft and obtain a new approval"
            .into(),
    )
}

impl LineMcpServer {
    fn send_store(&self) -> Result<Store, ToolFailure> {
        Store::open(self.policy_path.as_deref().ok_or_else(failure)?)
    }
    pub(crate) fn persist_draft(&self, record: &DraftRecord) -> Result<String, ToolFailure> {
        let owner = self.client_identity().0;
        let marker = marker(&record.raw)?;
        let id = format!(
            "draft-{}",
            sha256_hex(&json!([record.account_id.as_str(), owner, marker]).to_string())
        );
        self.send_store()?.save_draft(&id, &owner, record)?;
        Ok(id)
    }
    pub(crate) fn durable_draft(&self, id: &str) -> Result<DraftRecord, ToolFailure> {
        self.send_store()?.draft(id, &self.client_identity().0)
    }
    pub(crate) fn prepare_submission(
        &self,
        draft_id: &str,
        record: &DraftRecord,
    ) -> Result<Value, ToolFailure> {
        let store = self.send_store()?;
        let owner = self.client_identity().0;
        let operation_id = format!("send-{}", sha256_hex(&json!([draft_id, owner]).to_string()));
        let _lock = store.lock(&operation_id)?;
        let mut entry = if store.path(&operation_id)?.exists() {
            store.read::<Entry>(&operation_id)?
        } else {
            Entry {
                operation_id: operation_id.clone(),
                account_id: record.account_id.to_string(),
                owner,
                draft_id: draft_id.into(),
                fingerprint: fingerprint(record)?,
                code: crate::confirmation_code(&operation_id),
                expires_at: now_secs() + crate::PENDING_TTL_SECONDS,
                submission: Submission::Prepared,
                copy: CopyStatus::Pending,
                strategy: CopyStrategy::Imap,
                mailbox: None,
                copy_warning: None,
            }
        };
        if entry.fingerprint != fingerprint(record)? {
            return Err(changed());
        }
        if entry.submission == Submission::Prepared && entry.expires_at < now_secs() {
            entry.expires_at = now_secs() + crate::PENDING_TTL_SECONDS;
        }
        store.write(&operation_id, &entry)?;
        Ok(
            json!({"pending_action_id":operation_id,"operation_id":operation_id,"confirmation_code":entry.code,
            "expires_in_seconds":entry.expires_at.saturating_sub(now_secs()),"submission_status":entry.submission,
            "preview":format!("Send to {}{}",record.recipients.join(", "), if record.attachments.is_empty() { String::new() } else { format!(" with {} attachment(s)", record.attachments.len()) }),"from":record.from,"recipients":record.recipients,
            "subject":record.subject,"attachments":crate::attachment_summaries_json(&record.attachments),
            "attachment_count":record.attachments.len(),"total_attachment_bytes":crate::total_attachment_bytes(&record.attachments)}),
        )
    }
    pub(crate) fn confirm_submission(
        &self,
        operation_id: &str,
        code: &str,
        app: bool,
    ) -> Result<Value, ToolFailure> {
        let store = self.send_store()?;
        let _lock = store.lock(operation_id)?;
        let mut entry: Entry = store.read(operation_id)?;
        if !self.may_access_submission(&entry, app) {
            return Err(ToolFailure::InvalidParams(
                "send belongs to a different client".into(),
            ));
        }
        if entry.code != code {
            return Err(ToolFailure::InvalidParams(
                "confirmation_code does not match the prepared action".into(),
            ));
        }
        let account = AccountId::new(&entry.account_id);
        let (engine, _) = self.runtime_for(&account).map_err(ToolFailure::Io)?;
        engine
            .authorize(&account, Capability::Send)
            .map_err(ToolFailure::Core)?;
        if !self.origin_allowed(&entry)? {
            return Err(ToolFailure::Core(CoreError::AccountNotFound(account)));
        }
        let record = store.draft(&entry.draft_id, &entry.owner)?;
        if entry.fingerprint != fingerprint(&record)? {
            return Err(changed());
        }
        if record.account_id != account
            || self.account_email(&account).as_deref() != Some(record.from.as_str())
        {
            return Err(changed());
        }
        if entry.submission == Submission::Submitting {
            entry.submission = Submission::Unknown;
            store.write(operation_id, &entry)?;
        }
        if entry.submission == Submission::Prepared {
            if now_secs() > entry.expires_at {
                return Err(ToolFailure::Core(CoreError::PendingActionExpired(
                    operation_id.into(),
                )));
            }
            let Some((config, oauth)) = self.smtp_facts(&account) else {
                return Err(ToolFailure::Io(
                    "no SMTP is configured for this account — finish setting it up in TorroMail"
                        .into(),
                ));
            };
            validate_message(&record)?;
            let document = self
                .document_accounts()
                .map_err(ToolFailure::Io)?
                .unwrap_or_default()
                .into_iter()
                .find(|document| document.policy.account_id() == &account)
                .ok_or_else(failure)?;
            entry.strategy = document.sent_copy_strategy;
            entry.submission = Submission::Submitting;
            store.write(operation_id, &entry)?; // fsync BEFORE any SMTP attempt
            let result = match &self.smtp_override {
                Some(send) => send(&record.from, &record.recipients, &record.raw),
                None => crate::send_over_smtp(&config, oauth.as_ref(), &record),
            };
            entry.submission = match result {
                Ok(()) => Submission::Accepted,
                Err(SubmissionError::NotAccepted(_)) => Submission::NotAccepted,
                Err(SubmissionError::Unknown(_)) => Submission::Unknown,
            };
            entry.copy = match entry.strategy {
                CopyStrategy::Imap => CopyStatus::Pending,
                CopyStrategy::Provider => CopyStatus::Provider,
                CopyStrategy::None => CopyStatus::Skipped,
            };
            // If this write fails, the old Submitting checkpoint remains:
            // fail closed on restart and explicitly report known acceptance now.
            if store.write(operation_id, &entry).is_err() {
                let mut result = outcome(&entry);
                result["warning"] = json!(
                    "SMTP outcome could not be committed to disk; do not resubmit. Inspect this operation before taking further action."
                );
                return Ok(result);
            }
        }
        if entry.submission == Submission::Accepted
            && self.save_sent_copy(&store, &mut entry, &record).is_err()
        {
            entry.copy_warning = Some("Accepted by SMTP; copy recovery could not be recorded. Check local storage; do not resubmit.".into());
        }
        let mut payload = outcome(&entry);
        payload["recipients"] = json!(record.recipients.len());
        payload["attachment_count"] = json!(record.attachments.len());
        payload["total_attachment_bytes"] =
            json!(crate::total_attachment_bytes(&record.attachments));
        Ok(payload)
    }
    fn origin_allowed(&self, entry: &Entry) -> Result<bool, ToolFailure> {
        let Some(document) = self.document().map_err(ToolFailure::Io)? else {
            return Ok(true);
        };
        let Some(clients) = document.clients else {
            return Ok(true);
        };
        Ok(clients
            .into_iter()
            .find(|client| client.id == entry.owner)
            .is_some_and(|client| match client.account_access {
                crate::policy_document::ClientAccountAccess::All => true,
                crate::policy_document::ClientAccountAccess::Selected(ids) => {
                    ids.contains(&entry.account_id)
                }
            }))
    }
    fn may_access_submission(&self, entry: &Entry, app: bool) -> bool {
        let owner = self.client_identity().0;
        owner == entry.owner || (app && owner == "torromail-app")
    }
    fn save_sent_copy(
        &self,
        store: &Store,
        entry: &mut Entry,
        record: &DraftRecord,
    ) -> Result<(), ToolFailure> {
        if !matches!(entry.copy, CopyStatus::Pending | CopyStatus::Appending) {
            return Ok(());
        }
        // Recheck grants and send policy, including during background recovery.
        if !self.origin_allowed(entry)? {
            return Err(ToolFailure::InvalidParams(
                "originating account grant was revoked".into(),
            ));
        }
        let account = &record.account_id;
        let (engine, facts) = self.runtime_for(account).map_err(ToolFailure::Io)?;
        engine
            .authorize(account, Capability::Send)
            .map_err(ToolFailure::Core)?;
        let documents = self
            .document_accounts()
            .map_err(ToolFailure::Io)?
            .unwrap_or_default();
        let document = documents
            .into_iter()
            .find(|document| document.policy.account_id() == account)
            .ok_or_else(failure)?;
        // Honor an explicit decision to stop making additional copies.
        if entry.copy == CopyStatus::Pending && document.sent_copy_strategy != CopyStrategy::Imap {
            entry.copy = if document.sent_copy_strategy == CopyStrategy::Provider {
                CopyStatus::Provider
            } else {
                CopyStatus::Skipped
            };
            return store.write(&entry.operation_id, entry);
        }
        entry.copy_warning = None;
        let copy = (|| {
            let mut provider = self
                .open_connection(account, facts)
                .map_err(ToolFailure::Core)?;
            let allow_append = entry.copy == CopyStatus::Pending;
            // An ambiguous append must be reconciled in its original mailbox.
            let chosen = entry
                .mailbox
                .as_deref()
                .or(document.mailbox_overrides.get(SpecialMailboxRole::Sent));
            let mailbox = match provider.ensure_special_mailbox(
                account,
                SpecialMailboxRole::Sent,
                chosen,
                document.allow_create_sent_mailbox,
            ) {
                Ok(mailbox) => mailbox,
                Err(error) => {
                    entry.copy_warning = Some(if let Some(chosen) = chosen {
                        format!(
                            "The selected Sent folder {chosen:?} is unavailable; repair the mapping in TorroMail. It has not been replaced."
                        )
                    } else {
                        "The Sent folder is unavailable; check IMAP and the account's folder creation setting.".into()
                    });
                    return Err(ToolFailure::Core(error));
                }
            };
            entry.mailbox = Some(mailbox.clone());
            entry.copy = CopyStatus::Appending;
            store.write(&entry.operation_id, entry)?;
            match provider.append_sent_verified(
                account,
                &mailbox,
                &record.raw,
                &marker(&record.raw)?,
                allow_append,
            ) {
                Ok(()) => entry.copy = CopyStatus::Saved,
                Err(CoreError::DraftFailure(DraftFailureKind::StorageFailed)) if allow_append => {
                    entry.copy = CopyStatus::Pending
                }
                Err(_) => {} // APPEND may have succeeded; only reconcile next time
            }
            store.write(&entry.operation_id, entry)
        })();
        // A known SMTP acceptance is never turned into a failed send by IMAP.
        if copy.is_err() && entry.copy_warning.is_none() {
            entry.copy_warning = Some(
                "The Sent copy is pending; check the IMAP connection. SMTP will not be repeated."
                    .into(),
            );
        }
        store.write(&entry.operation_id, entry)
    }
    /// Recovery is copy-only. Neither Prepared nor Unknown ever runs SMTP.
    pub fn retry_sent_copies(&self) -> Result<usize, String> {
        self.require_pairing()?;
        let store = self.send_store().map_err(|error| error.message())?;
        let mut saved = 0;
        for id in store.ids().map_err(|error| error.message())? {
            let Some(_lock) = store.try_lock(&id).map_err(|error| error.message())? else {
                continue;
            };
            let mut entry: Entry = store.read(&id).map_err(|error| error.message())?;
            if !self.may_access_submission(&entry, true)
                || !self
                    .origin_allowed(&entry)
                    .map_err(|error| error.message())?
            {
                continue;
            }
            let account = AccountId::new(&entry.account_id);
            if self.runtime_for(&account).is_err() {
                continue;
            }
            if entry.submission == Submission::Submitting {
                entry.submission = Submission::Unknown;
                store.write(&id, &entry).map_err(|error| error.message())?;
            }
            if entry.submission != Submission::Accepted
                || !matches!(entry.copy, CopyStatus::Pending | CopyStatus::Appending)
            {
                continue;
            }
            let record = store
                .draft(&entry.draft_id, &entry.owner)
                .map_err(|error| error.message())?;
            if fingerprint(&record).map_err(|error| error.message())? != entry.fingerprint {
                continue;
            }
            if self.save_sent_copy(&store, &mut entry, &record).is_ok()
                && entry.copy == CopyStatus::Saved
            {
                saved += 1;
            }
        }
        Ok(saved)
    }
    fn require_pairing(&self) -> Result<(), String> {
        match self.client_gate()? {
            crate::ClientGate::Allowed => Ok(()),
            crate::ClientGate::Refused(message) => Err(message.into()),
        }
    }
    /// Local app bridge uses the same journal, authorization and submission.
    pub fn send_action_request(&self, request: &Value) -> Result<Value, String> {
        self.require_pairing()?;
        let result = (|| -> Result<Value, ToolFailure> {
            let store = self.send_store()?;
            match request["action"].as_str() {
                Some("list") => {
                    let mut entries = Vec::new();
                    for id in store.ids()? {
                        let entry: Entry = store.read(&id)?;
                        if !self.may_access_submission(&entry, true)
                            || !self.origin_allowed(&entry)?
                            || self
                                .runtime_for(&AccountId::new(&entry.account_id))
                                .is_err()
                        {
                            continue;
                        }
                        let pending_approval = entry.submission == Submission::Prepared
                            && entry.expires_at >= now_secs();
                        let exception = entry.submission == Submission::Unknown
                            || entry.submission == Submission::Submitting
                            || entry.submission == Submission::NotAccepted
                            || (entry.submission == Submission::Accepted
                                && matches!(
                                    entry.copy,
                                    CopyStatus::Pending | CopyStatus::Appending
                                ));
                        if !pending_approval && !exception {
                            continue;
                        }
                        let record = store.draft(&entry.draft_id, &entry.owner)?;
                        if fingerprint(&record)? != entry.fingerprint {
                            continue;
                        }
                        entries.push(json!({"id":id,"accountID":entry.account_id,"code":entry.code,"from":record.from,"recipients":record.recipients,
                            "subject":record.subject,"body":record.body,"submissionStatus":entry.submission,"sentCopyStatus":entry.copy,"attachments":crate::attachment_summaries_json(&record.attachments),"expiresAt":entry.expires_at}));
                    }
                    Ok(json!(entries))
                }
                Some("confirm") => self.confirm_submission(
                    request["id"].as_str().unwrap_or_default(),
                    request["code"].as_str().unwrap_or_default(),
                    true,
                ),
                Some("reject") => {
                    let id = request["id"].as_str().unwrap_or_default();
                    let _lock = store.lock(id)?;
                    let mut entry: Entry = store.read(id)?;
                    if !self.may_access_submission(&entry, true)
                        || !self.origin_allowed(&entry)?
                        || self
                            .runtime_for(&AccountId::new(&entry.account_id))
                            .is_err()
                    {
                        return Err(failure());
                    }
                    if entry.submission == Submission::Prepared {
                        entry.submission = Submission::Rejected;
                        store.write(id, &entry)?;
                    }
                    Ok(outcome(&entry))
                }
                _ => Err(ToolFailure::InvalidParams("unknown send action".into())),
            }
        })();
        if request["action"].as_str() == Some("confirm")
            && let Ok(payload) = &result
        {
            let args = json!({"pending_action_id":request["id"]});
            self.record_audit(
                &json!({"params":{"name":"mail_confirm_action","arguments":args}}),
                &crate::json_rpc_text_result(&Value::Null, payload),
            );
        }
        result.map_err(|error| error.message())
    }
}
fn marker(message: &str) -> Result<String, ToolFailure> {
    message
        .lines()
        .find_map(|line| line.strip_prefix("Message-ID: ").map(str::to_owned))
        .ok_or_else(failure)
}
fn validate_message(record: &DraftRecord) -> Result<(), ToolFailure> {
    if record.raw.len() > crate::MAX_MESSAGE_BYTES
        || record.recipients.is_empty()
        || std::iter::once(&record.from)
            .chain(record.recipients.iter())
            .any(|address| {
                address.is_empty()
                    || !address.contains('@')
                    || address.contains(['\r', '\n', '<', '>'])
            })
    {
        return Err(ToolFailure::InvalidParams(
            "invalid message or SMTP envelope".into(),
        ));
    }
    marker(&record.raw)?;
    Ok(())
}
fn outcome(entry: &Entry) -> Value {
    let mut result = json!({"operation_id":entry.operation_id,"submission_status":entry.submission,
        "sent_copy_status":if entry.submission == Submission::Accepted { json!(entry.copy) } else { json!("not_applicable") },"sent_copy_mailbox":entry.mailbox,
        "delivery_confirmed":false,
        "status":match entry.submission { Submission::Accepted => "sent", Submission::Unknown | Submission::Submitting => "unknown", Submission::Prepared => "prepared", _ => "not_sent" }});
    if entry.submission == Submission::Accepted
        && matches!(entry.copy, CopyStatus::Pending | CopyStatus::Appending)
    {
        result["sent_copy_status"] = json!("pending");
        result["warning"] = json!(
            "Accepted by the SMTP server; Sent copy is pending. Acceptance does not confirm recipient delivery. Only the copy will be retried."
        );
    } else if entry.submission == Submission::Unknown {
        result["warning"] = json!(
            "SMTP acceptance is unknown; no automatic resubmission. Inspect the operation before explicitly preparing a new message."
        );
    }
    if let Some(warning) = &entry.copy_warning {
        result["warning"] = json!(warning);
    }
    result
}
