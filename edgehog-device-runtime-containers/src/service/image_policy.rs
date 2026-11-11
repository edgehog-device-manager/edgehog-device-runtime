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

//! Container image pull policy engine and service.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
    str::FromStr,
};

use aho_corasick::{
    AhoCorasick, Anchored, BuildError, Input, StartKind, automaton::OverlappingState,
};
use astarte_device_sdk::{
    AstarteData, astarte_device_error::Error as AstarteSdkError, properties::PropAccess,
    types::TypeError,
};
use tracing::{debug, error, instrument, warn};

use crate::{
    events::{
        DatastreamEvent,
        image_policy::{ImagePolicyEvent, ImagePolicyEventStatus},
    },
    properties::{
        Client,
        image_policy::{AvailableImagePolicy, ContainerPolicy, ImagePolicyStatus},
    },
    requests::image_policy::{ImagePolicy, ImagePolicyMapping, PolicyRequirement},
    service::ServiceError,
};

const DEFAULT_DOCKER_DOMAIN: &str = "docker.io";
const LEGACY_DOCKER_DOMAIN: &str = "index.docker.io";
const OFFICIAL_LIBRARY_NAMESPACE: &str = "library";

/// 32-byte SHA-256 digest formatted and parsed as a 64-character lowercase hexadecimal string.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Computes the SHA-256 digest of `data`.
    #[must_use]
    pub fn hash(data: impl AsRef<[u8]>) -> Self {
        let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, data.as_ref());
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(digest.as_ref());

        Self(bytes)
    }

    /// Returns the raw 32-byte SHA-256 digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Debug for Sha256Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sha256Digest({self})")
    }
}

impl Display for Sha256Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

/// Error returned when parsing a [`Sha256Digest`] from a hex string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, displaydoc::Display)]
pub enum Sha256DigestError {
    /// expected a 64-character lowercase hex SHA-256 digest
    InvalidHex,
}

impl FromStr for Sha256Digest {
    type Err = Sha256DigestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(Sha256DigestError::InvalidHex);
        }

        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes).map_err(|_| Sha256DigestError::InvalidHex)?;

        Ok(Self(bytes))
    }
}

impl TryFrom<&str> for Sha256Digest {
    type Error = Sha256DigestError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl TryFrom<AstarteData> for Sha256Digest {
    type Error = AstarteSdkError<TypeError>;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        Self::from_str(&value).map_err(|err| {
            error!(%err, value, "couldn't parse sha256 digest value");

            AstarteSdkError::with(TypeError::Conversion, "couldn't parse sha256 digest value")
                .set_ctx(value)
        })
    }
}

impl From<Sha256Digest> for AstarteData {
    fn from(value: Sha256Digest) -> Self {
        AstarteData::String(value.to_string())
    }
}

/// Error returned when validating or building container image policies.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum PolicyError {
    /// repository policy scope cannot be empty
    EmptyRepository,
    /// invalid repository policy scope `{0}`
    InvalidRepository(String),
    /// repository scope hash `{expected}` does not match policy id `{actual}`
    RegistryIdMismatch {
        /// Expected SHA-256 hash of the raw repository string.
        expected: Sha256Digest,
        /// Actual registry id from the Astarte endpoint path.
        actual: Sha256Digest,
    },
    /// duplicate policy for normalized repository scope `{0}`
    DuplicateRepository(String),
    /// unrecognized image policy requirement `{0}`
    InvalidPolicyType(String),
    /// SignedBy image policy is not supported yet
    UnsupportedSignedBy,
    /// invalid image policy keyId `{0}`
    InvalidKeyId(String),
    /// couldn't build policy prefix matcher
    BuildMatcher(#[from] BuildError),
}

/// Snapshot of the previous policy matcher returned when [`ImagePolicyService`] rebuilds the
/// active matcher, allowing callers to detect which pulled images changed verification status.
#[derive(Debug)]
pub(crate) struct PolicySnapshot {
    default: ContainerPolicy,
    matcher: PolicyMatcher,
}

impl PolicySnapshot {
    /// Evaluates whether `image_reference` was allowed under this snapshot.
    pub(crate) fn is_allowed(&self, image_reference: &str) -> bool {
        let canonical = normalize_image_reference(image_reference);

        self.matcher
            .match_canonical(&canonical)
            .unwrap_or(self.default)
            .is_allowed()
    }
}

/// Manages dynamic container image pull policies and evaluates image references.
#[derive(Debug, Default)]
pub struct ImagePolicyService {
    default: ContainerPolicy,
    table: PolicyTable,
    matcher: PolicyMatcher,
}

impl ImagePolicyService {
    /// Creates a new [`ImagePolicyService`] with the given fallback `default` policy.
    #[must_use]
    pub fn new(default: ContainerPolicy) -> Self {
        Self {
            default,
            table: PolicyTable::default(),
            matcher: PolicyMatcher::default(),
        }
    }

    /// Loads stored `ImagePolicy` properties from Astarte, reconciles `AvailableImagePolicies`,
    /// and returns `Ok(true)` if any complete policy rule was applied.
    #[instrument(skip_all)]
    pub(crate) async fn sync_image_policies<D>(
        &mut self,
        device: &mut D,
    ) -> Result<bool, ServiceError>
    where
        D: Client + PropAccess + Send + Sync + 'static,
    {
        let events = ImagePolicy::fetch(device).await?;
        let mut available_policies = AvailableImagePolicy::fetch(device).await?;

        let mut rebuild = false;
        let mut error_ids = HashSet::new();

        for event in events {
            let registry_id = event.registry_id;
            // Policy is still present in Astarte; remove it from the set of stale properties to unset.
            available_policies.remove(&registry_id);

            match self.table.apply_event(event) {
                Ok(outcome) => rebuild |= outcome.is_changed(),
                Err(error) => {
                    warn!(
                        error = format!("{:#}", eyre::Report::new(error)),
                        %registry_id,
                        "ignoring invalid stored ImagePolicy rule"
                    );

                    error_ids.insert(registry_id);
                }
            }
        }

        let mut applied = false;
        if rebuild {
            match self.table.build_matcher() {
                Ok(matcher) => {
                    self.matcher = matcher;
                    applied = true;
                }
                Err(error) => {
                    error!(
                        %error,
                        "couldn't rebuild the image policy matcher, keeping the stale one"
                    );
                }
            }
        }

        // Unset stale AvailableImagePolicies properties that are no longer active
        for policy_id in available_policies {
            if self.table.status(&policy_id).is_none() && !error_ids.contains(&policy_id) {
                AvailableImagePolicy::new(policy_id).unset(device).await;
            }
        }

        // update status of AvailableImagePolicies
        for (id, status) in self.table.statuses() {
            if !error_ids.contains(&id) {
                AvailableImagePolicy::new(id).send(device, status).await;
            }
        }

        // set error status
        for id in error_ids {
            AvailableImagePolicy::new(id)
                .send(device, ImagePolicyStatus::Error)
                .await;
        }

        Ok(applied)
    }

    /// Applies a live [`ImagePolicy`] update, publishes the resulting status and datastream event
    /// to Astarte, and returns the previous [`PolicySnapshot`] if the enforced matcher changed.
    #[instrument(skip_all, fields(registry_id = %policy.registry_id))]
    pub(crate) async fn handle_image_policy<D>(
        &mut self,
        device: &mut D,
        policy: ImagePolicy,
    ) -> Option<PolicySnapshot>
    where
        D: Client + Send + Sync + 'static,
    {
        let registry_id = policy.registry_id;

        let (outcome, prev_snapshot) = match self.apply_event(policy) {
            Ok(res) => res,
            Err(error) => {
                warn!(
                    error = format!("{error:#}"),
                    %registry_id,
                    "couldn't apply image policy"
                );

                AvailableImagePolicy::new(registry_id)
                    .send(device, ImagePolicyStatus::Error)
                    .await;

                ImagePolicyEvent::with_error(ImagePolicyEventStatus::Error, &error)
                    .send(&registry_id, device)
                    .await;

                return None;
            }
        };

        match outcome {
            PolicyOutcome::Applied => {
                AvailableImagePolicy::new(registry_id)
                    .send(device, ImagePolicyStatus::Present)
                    .await;

                ImagePolicyEvent::with_status(ImagePolicyEventStatus::Created)
                    .send(&registry_id, device)
                    .await;
            }
            PolicyOutcome::Incomplete => {
                AvailableImagePolicy::new(registry_id)
                    .send(device, ImagePolicyStatus::Missing)
                    .await;
            }
            PolicyOutcome::Unset => {
                AvailableImagePolicy::new(registry_id).unset(device).await;

                ImagePolicyEvent::with_status(ImagePolicyEventStatus::Removed)
                    .send(&registry_id, device)
                    .await;
            }
            PolicyOutcome::Unchanged => {
                if let Some(status) = self.status(&registry_id) {
                    AvailableImagePolicy::new(registry_id)
                        .send(device, status)
                        .await;
                }
            }
        }

        prev_snapshot
    }

    /// Applies a single [`ImagePolicy`] event and rebuilds the [`PolicyMatcher`] if any complete
    /// rule changed, returning the [`PolicyOutcome`] and previous [`PolicySnapshot`] (when rebuilt).
    pub(crate) fn apply_event(
        &mut self,
        event: ImagePolicy,
    ) -> Result<(PolicyOutcome, Option<PolicySnapshot>), PolicyError> {
        let outcome = self.table.apply_event(event)?;

        if !outcome.is_changed() {
            return Ok((outcome, None));
        }

        let matcher = self.table.build_matcher().inspect_err(|error| {
            error!(
                %error,
                "couldn't rebuild the image policy matcher, keeping the stale one"
            );
        })?;

        let prev = std::mem::replace(&mut self.matcher, matcher);

        Ok((
            outcome,
            Some(PolicySnapshot {
                default: self.default,
                matcher: prev,
            }),
        ))
    }

    /// Returns the current [`ImagePolicyStatus`] for `id`, or `None` if all fields are unset.
    pub(crate) fn status(&self, id: &Sha256Digest) -> Option<ImagePolicyStatus> {
        self.table.status(id)
    }

    /// Evaluates the effective [`ContainerPolicy`] for the given image reference.
    pub(crate) fn policy_for(&self, image_reference: &str) -> ContainerPolicy {
        let canonical = normalize_image_reference(image_reference);
        let policy = self
            .matcher
            .match_canonical(&canonical)
            .unwrap_or(self.default);

        debug!(
            image_reference,
            canonical_repository = %canonical,
            ?policy,
            "evaluated container image policy"
        );

        policy
    }

    /// Returns `true` if pulling `image_reference` is permitted by the current policies.
    pub(crate) fn is_allowed(&self, image_reference: &str) -> bool {
        self.policy_for(image_reference).is_allowed()
    }
}

/// Outcome of applying an [`ImagePolicy`] event to a [`PolicyTable`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOutcome {
    /// The policy rule is complete and has been applied.
    Applied,
    /// The previously enforced rule became incomplete: it no longer contributes to the
    /// matcher.
    Incomplete,
    /// All the fields of the policy rule have been unset.
    Unset,
    /// The policy rule didn't change its enforced `(repository, policy)` contribution.
    Unchanged,
}

impl PolicyOutcome {
    const fn is_changed(self) -> bool {
        match self {
            PolicyOutcome::Incomplete | PolicyOutcome::Applied | PolicyOutcome::Unset => true,
            PolicyOutcome::Unchanged => false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CompletePolicy<'a> {
    repository: Cow<'a, str>,
    policy: ContainerPolicy,
}

impl<'a> CompletePolicy<'a> {
    fn new(repository: &'a str, policy: ContainerPolicy) -> Self {
        Self {
            repository: Cow::Borrowed(repository),
            policy,
        }
    }

    fn into_static(self) -> CompletePolicy<'static> {
        CompletePolicy {
            repository: Cow::Owned(self.repository.into_owned()),
            policy: self.policy,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct TempPolicy {
    repository: Option<String>,
    key_id: Option<Sha256Digest>,
    ty: Option<PolicyRequirement>,
}

impl TempPolicy {
    const fn is_empty(&self) -> bool {
        self.repository.is_none() && self.key_id.is_none() && self.ty.is_none()
    }

    fn complete_policy(&self) -> Option<CompletePolicy<'_>> {
        let repository = self.repository.as_deref()?;
        let policy = match self.ty? {
            PolicyRequirement::Reject => ContainerPolicy::Reject,
            PolicyRequirement::SignedBy => return None,
        };

        Some(CompletePolicy::new(repository, policy))
    }

    fn status(&self) -> ImagePolicyStatus {
        if self.complete_policy().is_some() {
            ImagePolicyStatus::Present
        } else {
            ImagePolicyStatus::Missing
        }
    }
}

#[derive(Debug, Default)]
struct PolicyMatcher {
    ac: Option<AhoCorasick>,
    policies: Vec<ContainerPolicy>,
}

impl PolicyMatcher {
    fn build<'a, I>(entries: I) -> Result<Self, PolicyError>
    where
        I: IntoIterator<Item = CompletePolicy<'a>>,
    {
        let (patterns, policies): (Vec<Cow<'a, str>>, Vec<ContainerPolicy>) = entries
            .into_iter()
            .map(|c| (c.repository, c.policy))
            .unzip();

        if patterns.is_empty() {
            return Ok(Self::default());
        }

        let ac = AhoCorasick::builder()
            .start_kind(StartKind::Anchored)
            .build(patterns.iter().map(|s| s.as_bytes()))?;

        Ok(Self {
            ac: Some(ac),
            policies,
        })
    }

    fn match_canonical(&self, canonical_repo: &str) -> Option<ContainerPolicy> {
        let ac = self.ac.as_ref()?;

        let input = Input::new(canonical_repo).anchored(Anchored::Yes);
        let mut state = OverlappingState::start();
        let mut best_match: Option<(usize, usize)> = None;

        loop {
            if let Err(error) = ac.try_find_overlapping(input.clone(), &mut state) {
                error!(%error, "unexpected error searching image policy matcher");
                return None;
            }

            let Some(mat) = state.get_match() else {
                break;
            };

            debug_assert_eq!(mat.start(), 0);

            let end = mat.end();
            if end == canonical_repo.len() || canonical_repo.as_bytes().get(end) == Some(&b'/') {
                match best_match {
                    Some((best_end, _)) if best_end >= end => {}
                    _ => {
                        best_match = Some((end, mat.pattern().as_usize()));
                    }
                }
            }
        }

        best_match.and_then(|(_, pattern_idx)| self.policies.get(pattern_idx).copied())
    }
}

/// Accumulator table of dynamic container image policies indexed by [`Sha256Digest`].
#[derive(Debug, Default)]
struct PolicyTable {
    map: HashMap<Sha256Digest, TempPolicy>,
}

enum ValidatedMapping {
    Repository(Option<String>),
    Type(Option<PolicyRequirement>),
    KeyId(Option<Sha256Digest>),
}

impl PolicyTable {
    /// Applies a single [`ImagePolicy`] event to `self.map` without rebuilding the matcher.
    ///
    /// All fields are validated before mutating the table, so an invalid update leaves the
    /// last-known-good state untouched.
    pub(crate) fn apply_event(&mut self, event: ImagePolicy) -> Result<PolicyOutcome, PolicyError> {
        let ImagePolicy {
            registry_id: id,
            mapping,
        } = event;

        let validated = match mapping {
            ImagePolicyMapping::Repository(Some(raw_scope)) => {
                let repository = self.validate_repository_scope(id, &raw_scope)?;
                ValidatedMapping::Repository(Some(repository))
            }
            ImagePolicyMapping::Repository(None) => ValidatedMapping::Repository(None),
            ImagePolicyMapping::Type(Some(raw_ty)) => {
                let req = PolicyRequirement::from_str(&raw_ty)
                    .map_err(|()| PolicyError::InvalidPolicyType(raw_ty))?;

                if req == PolicyRequirement::SignedBy {
                    warn!(%id, "SignedBy image policy is not supported yet");
                    return Err(PolicyError::UnsupportedSignedBy);
                }

                ValidatedMapping::Type(Some(req))
            }
            ImagePolicyMapping::Type(None) => ValidatedMapping::Type(None),
            ImagePolicyMapping::KeyId(Some(raw_key_id)) => {
                let key_id = Sha256Digest::from_str(&raw_key_id)
                    .map_err(|_| PolicyError::InvalidKeyId(raw_key_id))?;
                warn!(%id, %key_id, "key id mapping is currently not supported");
                ValidatedMapping::KeyId(Some(key_id))
            }
            ImagePolicyMapping::KeyId(None) => ValidatedMapping::KeyId(None),
        };

        let entry = self.map.entry(id).or_default();
        let prev_complete = entry.complete_policy().map(CompletePolicy::into_static);

        match validated {
            ValidatedMapping::Repository(repository) => {
                entry.repository = repository;
            }
            ValidatedMapping::Type(req) => {
                entry.ty = req;
            }
            ValidatedMapping::KeyId(key_id) => {
                entry.key_id = key_id;
            }
        }

        if entry.is_empty() {
            self.map.remove(&id);

            return Ok(PolicyOutcome::Unset);
        }

        let new_complete = entry.complete_policy();

        let outcome = match (new_complete, prev_complete) {
            (Some(new), Some(prev)) if new == prev => PolicyOutcome::Unchanged,
            (Some(_), _) => PolicyOutcome::Applied,
            (None, Some(_)) => PolicyOutcome::Incomplete,
            (None, None) => PolicyOutcome::Unchanged,
        };

        Ok(outcome)
    }

    fn validate_repository_scope(
        &self,
        id: Sha256Digest,
        raw_scope: &str,
    ) -> Result<String, PolicyError> {
        let normalized = normalize_policy_scope(raw_scope).inspect_err(|error| {
            debug!(%id, %error, "invalid repository scope in ImagePolicy");
        })?;

        let expected = Sha256Digest::hash(raw_scope.as_bytes());
        if expected != id {
            debug!(%expected, actual = %id, "registry_id does not match sha256 of repository scope");
            return Err(PolicyError::RegistryIdMismatch {
                expected,
                actual: id,
            });
        }

        if self.map.iter().any(|(&other_id, entry)| {
            other_id != id && entry.repository.as_deref() == Some(&normalized)
        }) {
            return Err(PolicyError::DuplicateRepository(normalized));
        }

        Ok(normalized)
    }

    fn build_matcher(&self) -> Result<PolicyMatcher, PolicyError> {
        PolicyMatcher::build(self.map.values().filter_map(TempPolicy::complete_policy))
    }

    fn status(&self, id: &Sha256Digest) -> Option<ImagePolicyStatus> {
        self.map.get(id).map(TempPolicy::status)
    }

    fn statuses(&self) -> impl Iterator<Item = (Sha256Digest, ImagePolicyStatus)> + '_ {
        self.map.iter().map(|(&id, entry)| (id, entry.status()))
    }
}

/// Normalizes and validates a policy repository scope (e.g. `docker.io`, `quay.io/my-org`,
/// `registry.example.com:5000/team/app`) for prefix matching.
fn normalize_policy_scope(scope: &str) -> Result<String, PolicyError> {
    let without_trailing = scope.trim().trim_end_matches('/');
    if without_trailing.is_empty() {
        return Err(PolicyError::EmptyRepository);
    }

    // Reject leading/trailing whitespace on non-empty scopes so raw hash matches canonical syntax,
    // as well as schemes, leading slashes, double slashes, digests, and internal whitespace.
    let trimmed = scope.trim_end_matches('/');
    if trimmed != without_trailing
        || trimmed.starts_with('/')
        || trimmed.contains("//")
        || trimmed.contains("://")
        || trimmed.contains('@')
        || trimmed.chars().any(char::is_whitespace)
    {
        return Err(PolicyError::InvalidRepository(scope.to_string()));
    }

    // Validate host[:port] and ensure no `:tag` appears in repository path components.
    match trimmed.split_once('/') {
        Some((host_part, repo_part)) => {
            if !is_valid_host_or_namespace(host_part) || repo_part.contains(':') {
                return Err(PolicyError::InvalidRepository(scope.to_string()));
            }
        }
        None => {
            if !is_valid_registry_domain(trimmed) {
                return Err(PolicyError::InvalidRepository(scope.to_string()));
            }
        }
    }

    if trimmed == LEGACY_DOCKER_DOMAIN {
        Ok(DEFAULT_DOCKER_DOMAIN.to_string())
    } else if let Some(rest) = trimmed.strip_prefix("index.docker.io/") {
        Ok(format!("{DEFAULT_DOCKER_DOMAIN}/{rest}"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn is_valid_host_or_namespace(part: &str) -> bool {
    if part.is_empty() {
        return false;
    }

    match part.split_once(':') {
        None => true,
        Some((host, port)) => !host.is_empty() && u16::from_str(port).is_ok(),
    }
}

fn is_valid_registry_domain(domain: &str) -> bool {
    match domain.split_once(':') {
        Some((host, port)) => !host.is_empty() && u16::from_str(port).is_ok(),
        None => domain == "localhost" || domain.contains('.'),
    }
}

/// Normalizes and canonicalizes a Docker/OCI image reference into `<domain>/<repository>`
/// following `github.com/distribution/reference/normalize.go` (`splitDockerDomain`),
/// stripping any `:tag` and `@digest` suffixes.
pub(crate) fn normalize_image_reference(reference: &str) -> String {
    let reference = reference.trim();

    // 1. Strip @digest if present
    let without_digest = reference
        .split_once('@')
        .map_or(reference, |(before_digest, _)| before_digest);

    // 2. Strip :tag from the final path component only (preserving host:port)
    let without_tag = match without_digest.rfind('/') {
        Some(slash_idx) => match without_digest[slash_idx + 1..].split_once(':') {
            Some((repo_leaf, _tag)) => &without_digest[..slash_idx + 1 + repo_leaf.len()],
            None => without_digest,
        },
        None => without_digest
            .split_once(':')
            .map_or(without_digest, |(repo_name, _tag)| repo_name),
    };

    // 3. Split domain and remainder following distribution/reference splitDockerDomain
    let (mut domain, remainder) = match without_tag.split_once('/') {
        None => (DEFAULT_DOCKER_DOMAIN, without_tag),
        Some((first, _rest))
            if !first.contains(['.', ':'])
                && first != "localhost"
                && !first.chars().any(|c| c.is_ascii_uppercase()) =>
        {
            (DEFAULT_DOCKER_DOMAIN, without_tag)
        }
        Some((first, rest)) => (first, rest),
    };

    if domain == LEGACY_DOCKER_DOMAIN {
        domain = DEFAULT_DOCKER_DOMAIN;
    }

    if domain == DEFAULT_DOCKER_DOMAIN && !remainder.contains('/') {
        format!("{domain}/{OFFICIAL_LIBRARY_NAMESPACE}/{remainder}")
    } else {
        format!("{domain}/{remainder}")
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::FromEvent;
    use astarte_device_sdk::astarte_interfaces::schema::Ownership;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::{SqliteStore, StoredProp};
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use astarte_device_sdk_mock::mockall::Sequence;
    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use crate::requests::image_policy::tests::image_policy_event;

    use super::*;

    fn repository_event(id: Sha256Digest, repo: Option<&str>) -> ImagePolicy {
        ImagePolicy::from_event(image_policy_event(
            &format!("/policy/{id}/repository"),
            repo,
        ))
        .unwrap()
    }

    fn type_event(id: Sha256Digest, ty: Option<&str>) -> ImagePolicy {
        ImagePolicy::from_event(image_policy_event(&format!("/policy/{id}/type"), ty)).unwrap()
    }

    fn key_id_event(id: Sha256Digest, key_id: Option<&str>) -> ImagePolicy {
        ImagePolicy::from_event(image_policy_event(&format!("/policy/{id}/keyId"), key_id)).unwrap()
    }

    #[test]
    fn should_hash_format_and_parse_sha256_digest() {
        let digest = Sha256Digest::hash(b"quay.io/my-org");
        // Copy semantics check
        let copied = digest;
        assert_eq!(digest, copied);

        let hex_str = digest.to_string();
        assert_eq!(hex_str.len(), 64);
        assert!(
            hex_str
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        );

        let parsed = Sha256Digest::from_str(&hex_str).unwrap();
        assert_eq!(parsed, digest);

        // Uppercase hex, wrong length, and non-hex characters must be rejected
        assert!(Sha256Digest::from_str(&hex_str.to_ascii_uppercase()).is_err());
        assert!(Sha256Digest::from_str(&hex_str[..63]).is_err());
        assert!(Sha256Digest::from_str(&format!("{}g", &hex_str[..63])).is_err());
    }

    #[rstest]
    #[case("postgres:15", "docker.io/library/postgres")]
    #[case("alpine", "docker.io/library/alpine")]
    #[case("docker.io/alpine:3.18", "docker.io/library/alpine")]
    #[case(
        "docker.io/library/nginx:stable-alpine-slim",
        "docker.io/library/nginx"
    )]
    #[case("index.docker.io/alpine:latest", "docker.io/library/alpine")]
    #[case("index.docker.io/myorg/myapp:1.0", "docker.io/myorg/myapp")]
    #[case("myorg/myapp:v1", "docker.io/myorg/myapp")]
    #[case("localhost/myapp:v1", "localhost/myapp")]
    #[case("localhost:5000/team/app:1.0", "localhost:5000/team/app")]
    #[case(
        "registry.example.com:5000/team/app",
        "registry.example.com:5000/team/app"
    )]
    #[case(
        "registry.example.com:5000/team/app:1.2.3@sha256:0123456789abcdef",
        "registry.example.com:5000/team/app"
    )]
    #[case("quay.io/my-org/app@sha256:0123456789abcdef", "quay.io/my-org/app")]
    fn should_normalize_image_reference(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(normalize_image_reference(input), expected);
    }

    #[test]
    fn should_fallback_to_default_policy() {
        let allow_service = ImagePolicyService::default();
        assert_eq!(
            allow_service.policy_for("postgres:15"),
            ContainerPolicy::InsecureAcceptAnything
        );
        assert!(allow_service.is_allowed("postgres:15"));

        let reject_service = ImagePolicyService::new(ContainerPolicy::Reject);
        assert_eq!(
            reject_service.policy_for("postgres:15"),
            ContainerPolicy::Reject
        );
        assert!(!reject_service.is_allowed("postgres:15"));
    }

    #[rstest]
    #[case("")]
    #[case("   /")]
    fn should_reject_empty_repository_scope(#[case] scope: &str) {
        assert!(matches!(
            normalize_policy_scope(scope),
            Err(PolicyError::EmptyRepository)
        ));
    }

    #[rstest]
    #[case("quay.io/my-org/app:latest")]
    #[case("quay.io/my-org/app:15")]
    #[case("quay.io/my-org/app@sha256:0123456789abcdef")]
    #[case("https://quay.io/my-org")]
    #[case("/quay.io/my-org")]
    #[case("quay.io//my-org")]
    #[case("quay.io/my org")]
    #[case("alpine:latest")]
    #[case("postgres")]
    #[case(" docker.io")]
    fn should_reject_malformed_repository_scopes(#[case] scope: &str) {
        assert!(matches!(
            normalize_policy_scope(scope),
            Err(PolicyError::InvalidRepository(_))
        ));
    }

    #[rstest]
    #[case("docker.io", "docker.io")]
    #[case("index.docker.io", "docker.io")]
    #[case("index.docker.io/my-org", "docker.io/my-org")]
    #[case("quay.io/my-org/", "quay.io/my-org")]
    #[case("localhost", "localhost")]
    #[case("localhost:5000", "localhost:5000")]
    #[case("localhost:5000/team/app", "localhost:5000/team/app")]
    #[case(
        "registry.example.com:5000/team/app",
        "registry.example.com:5000/team/app"
    )]
    fn should_normalize_valid_repository_scopes(#[case] scope: &str, #[case] expected: &str) {
        assert_eq!(normalize_policy_scope(scope).unwrap(), expected);
    }

    #[test]
    fn should_match_anchored_longest_prefix_on_path_boundaries() {
        let matcher = PolicyMatcher::build([
            CompletePolicy::new("quay.io", ContainerPolicy::InsecureAcceptAnything),
            CompletePolicy::new("quay.io/my-org", ContainerPolicy::Reject),
            CompletePolicy::new(
                "quay.io/my-org/trusted-app",
                ContainerPolicy::InsecureAcceptAnything,
            ),
            CompletePolicy::new(
                "docker.io/library/postgres",
                ContainerPolicy::InsecureAcceptAnything,
            ),
        ])
        .unwrap();

        let eval = |img: &str| {
            let canonical = normalize_image_reference(img);
            matcher
                .match_canonical(&canonical)
                .unwrap_or(ContainerPolicy::Reject)
        };

        // Longest prefix wins
        assert_eq!(
            eval("quay.io/other-org/app:1.0"),
            ContainerPolicy::InsecureAcceptAnything
        );
        assert_eq!(
            eval("quay.io/my-org/untrusted:1.0"),
            ContainerPolicy::Reject
        );
        assert_eq!(
            eval("quay.io/my-org/trusted-app:2.0"),
            ContainerPolicy::InsecureAcceptAnything
        );

        // Path-component boundary: "quay.io/my-org" must NOT match "quay.io/my-org-extra/app"
        assert_eq!(
            eval("quay.io/my-org-extra/app:1.0"),
            ContainerPolicy::InsecureAcceptAnything
        );

        // Path-component boundary: "docker.io/library/postgres" matches "postgres:15",
        // NOT "postgres-exporter:latest"
        assert_eq!(eval("postgres:15"), ContainerPolicy::InsecureAcceptAnything);
        assert_eq!(eval("postgres-exporter:latest"), ContainerPolicy::Reject);

        // Anchored start: "evil.com/quay.io/other-org/app:1.0" must NOT match "quay.io"
        assert_eq!(
            eval("evil.com/quay.io/other-org/app:1.0"),
            ContainerPolicy::Reject
        );
    }

    #[test]
    fn should_assemble_incremental_events_and_handle_unsets() {
        let mut service = ImagePolicyService::new(ContainerPolicy::InsecureAcceptAnything);

        let h1 = Sha256Digest::hash(b"docker.io");
        let h2 = Sha256Digest::hash(b"ghcr.io/my-org");
        let dummy_key = Sha256Digest::hash(b"my-public-key").to_string();

        // 1. Receive only /repository -> incomplete, still allowed
        let (outcome, snap) = service
            .apply_event(repository_event(h1, Some("docker.io")))
            .unwrap();
        assert_eq!(outcome, PolicyOutcome::Unchanged);
        assert!(snap.is_none());
        assert!(service.is_allowed("postgres:15"));
        assert_eq!(service.status(&h1), Some(ImagePolicyStatus::Missing));

        // 2. Receive /type = Reject -> rule becomes complete and blocks docker.io
        let (outcome, snap) = service.apply_event(type_event(h1, Some("Reject"))).unwrap();
        assert_eq!(outcome, PolicyOutcome::Applied);
        assert!(snap.unwrap().is_allowed("postgres:15"));
        assert!(!service.is_allowed("postgres:15"));
        assert_eq!(service.status(&h1), Some(ImagePolicyStatus::Present));

        // 3. Receive a dynamic rule in reverse order (/type first, then /repository) and unset it
        service.apply_event(type_event(h2, Some("Reject"))).unwrap();
        assert!(service.is_allowed("ghcr.io/my-org/app:latest"));

        service
            .apply_event(repository_event(h2, Some("ghcr.io/my-org")))
            .unwrap();
        assert!(!service.is_allowed("ghcr.io/my-org/app:latest"));

        // Setting and unsetting keyId keeps track of non-empty entry properly
        let h3 = Sha256Digest::hash(b"registry.example.com");
        let (outcome, _) = service
            .apply_event(key_id_event(h3, Some(&dummy_key)))
            .unwrap();
        assert_eq!(outcome, PolicyOutcome::Unchanged);
        assert_eq!(service.status(&h3), Some(ImagePolicyStatus::Missing));
        let (outcome, _) = service.apply_event(key_id_event(h3, None)).unwrap();
        assert_eq!(outcome, PolicyOutcome::Unset);
        assert_eq!(service.status(&h3), None);

        // Unset h2 completely
        let (outcome, snap) = service.apply_event(type_event(h2, None)).unwrap();
        assert_eq!(outcome, PolicyOutcome::Incomplete);
        assert!(snap.is_some());
        assert!(service.is_allowed("ghcr.io/my-org/app:latest"));

        let (outcome, _) = service.apply_event(repository_event(h2, None)).unwrap();
        assert_eq!(outcome, PolicyOutcome::Unset);
        assert_eq!(service.status(&h2), None);
    }

    #[test]
    fn should_reject_signed_by_invalid_types_mismatched_hashes_and_duplicates() {
        let mut service = ImagePolicyService::new(ContainerPolicy::InsecureAcceptAnything);

        let docker_id = Sha256Digest::hash(b"docker.io");
        service
            .apply_event(repository_event(docker_id, Some("docker.io")))
            .unwrap();
        service
            .apply_event(type_event(docker_id, Some("Reject")))
            .unwrap();
        assert!(!service.is_allowed("postgres:15"));

        // 1. SignedBy must fail with UnsupportedSignedBy and leave the existing Reject rule enforced
        let err = service
            .apply_event(type_event(docker_id, Some("SignedBy")))
            .unwrap_err();
        assert!(matches!(err, PolicyError::UnsupportedSignedBy));
        assert!(!service.is_allowed("postgres:15"));

        // 2. InsecureAcceptAnything and unknown types are rejected on dynamic ImagePolicy
        let err = service
            .apply_event(type_event(docker_id, Some("InsecureAcceptAnything")))
            .unwrap_err();
        assert!(matches!(err, PolicyError::InvalidPolicyType(_)));

        let err = service
            .apply_event(type_event(docker_id, Some("Unknown")))
            .unwrap_err();
        assert!(matches!(err, PolicyError::InvalidPolicyType(_)));

        // 3. Invalid keyId (not a 64-char lowercase hex sha256)
        let err = service
            .apply_event(key_id_event(docker_id, Some("not-a-sha256")))
            .unwrap_err();
        assert!(matches!(err, PolicyError::InvalidKeyId(_)));

        // 4. registry_id != sha256(raw_scope)
        let wrong_id = Sha256Digest::hash(b"quay.io");
        let err = service
            .apply_event(repository_event(wrong_id, Some("ghcr.io")))
            .unwrap_err();
        assert!(matches!(err, PolicyError::RegistryIdMismatch { .. }));

        // 5. Duplicate normalized repository scope across distinct hashes ("index.docker.io" -> "docker.io")
        let legacy_docker_id = Sha256Digest::hash(b"index.docker.io");
        let err = service
            .apply_event(repository_event(legacy_docker_id, Some("index.docker.io")))
            .unwrap_err();
        assert!(matches!(err, PolicyError::DuplicateRepository(s) if s == "docker.io"));
    }

    #[tokio::test]
    async fn should_keep_error_status_on_partial_invalid_rule_during_sync() {
        let mut service = ImagePolicyService::new(ContainerPolicy::InsecureAcceptAnything);
        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        let bad_scope = "quay.io/my-org/app:latest";
        let bad_id = Sha256Digest::hash(bad_scope.as_bytes());
        let stale_id = Sha256Digest::hash(b"gcr.io/old-project");

        device
            .expect_interface_props()
            .once()
            .in_sequence(&mut seq)
            .with(predicate::eq("io.edgehog.devicemanager.apps.ImagePolicy"))
            .returning(move |_| {
                Ok(vec![
                    StoredProp {
                        interface: "io.edgehog.devicemanager.apps.ImagePolicy".to_string(),
                        path: format!("/policy/{bad_id}/repository"),
                        value: AstarteData::String(bad_scope.to_string()),
                        interface_major: 0,
                        ownership: Ownership::Server,
                    },
                    StoredProp {
                        interface: "io.edgehog.devicemanager.apps.ImagePolicy".to_string(),
                        path: format!("/policy/{bad_id}/type"),
                        value: AstarteData::String("Reject".to_string()),
                        interface_major: 0,
                        ownership: Ownership::Server,
                    },
                ])
            });

        device
            .expect_interface_props()
            .once()
            .in_sequence(&mut seq)
            .with(predicate::eq(
                "io.edgehog.devicemanager.apps.AvailableImagePolicies",
            ))
            .returning(move |_| {
                Ok(vec![StoredProp {
                    interface: "io.edgehog.devicemanager.apps.AvailableImagePolicies".to_string(),
                    path: format!("/{stale_id}/status"),
                    value: AstarteData::String("Present".to_string()),
                    interface_major: 0,
                    ownership: Ownership::Device,
                }])
            });

        // Must report Error once for bad_id (and NOT overwrite it with Missing afterwards)
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableImagePolicies"),
                predicate::eq(format!("/{bad_id}/status")),
                predicate::eq(AstarteData::from("Error")),
            )
            .returning(|_, _, _| Ok(()));

        // Stale policy must be unset
        device
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableImagePolicies"),
                predicate::eq(format!("/{stale_id}/status")),
            )
            .returning(|_, _| Ok(()));

        let applied = service.sync_image_policies(&mut device).await.unwrap();
        assert!(!applied);
    }
}
