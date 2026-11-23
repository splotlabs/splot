// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sealed compile-time SIMD backends and startup capability selection.
//!
//! Host capabilities are probed once per process via `fearless_simd` and
//! cached in an immutable `OnceLock`. Callers resolve a requested
//! [`SimdPolicy`] at session initialization into a concrete [`ResolvedBackend`]
//! holding a genuine SIMD capability token, keeping hot loops free of feature
//! queries and discriminator state.

use core::fmt;
use std::sync::OnceLock;

use fearless_simd::{Level, Simd};

mod sealed {
    pub trait Sealed {}
}

/// Requested SIMD execution policy resolved at initialization time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SimdPolicy {
    /// Selects the crate's verified default execution backend for this host.
    #[default]
    Auto,
    /// Requires 256-bit AVX2 support on x86-64.
    Avx2,
    /// Requires the full Ice Lake AVX-512 feature and OS-state bundle on x86-64.
    Avx512,
    /// Requires 128-bit NEON support on AArch64.
    Neon,
}

impl SimdPolicy {
    /// Stable lowercase identifier for diagnostics and telemetry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Avx2 => "avx2",
            Self::Avx512 => "avx512",
            Self::Neon => "neon",
        }
    }
}

impl fmt::Display for SimdPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Concrete backend chosen during startup selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SelectedBackend {
    /// x86-64 AVX2 backend.
    Avx2,
    /// x86-64 AVX-512 backend.
    Avx512,
    /// AArch64 NEON backend.
    Neon,
    /// Portable scalar-width fallback backend for other targets.
    Fallback,
}

impl SelectedBackend {
    /// Stable lowercase identifier for observation outside hot loops.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Avx2 => "avx2",
            Self::Avx512 => "avx512",
            Self::Neon => "neon",
            Self::Fallback => "fallback",
        }
    }
}

impl fmt::Display for SelectedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validated host SIMD capabilities probed from `fearless_simd`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuCapabilities {
    avx2: bool,
    avx512: bool,
    neon: bool,
}

impl CpuCapabilities {
    /// Whether the host CPU and OS support x86-64 AVX2.
    #[must_use]
    pub const fn has_avx2(self) -> bool {
        self.avx2
    }

    /// Whether the host CPU and OS pass `fearless_simd`'s full AVX-512 check.
    #[must_use]
    pub const fn has_avx512(self) -> bool {
        self.avx512
    }

    /// Whether the host supports AArch64 NEON.
    #[must_use]
    pub const fn has_neon(self) -> bool {
        self.neon
    }

    /// Whether [`SimdPolicy::Auto`] promotes to AVX-512 automatically.
    #[must_use]
    pub const fn avx512_qualified_for_auto(self) -> bool {
        false
    }

    #[cfg(test)]
    const fn from_flags(avx2: bool, avx512: bool, neon: bool) -> Self {
        Self { avx2, avx512, neon }
    }
}

impl fmt::Display for CpuCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "avx2={}, avx512={}, neon={}",
            self.avx2, self.avx512, self.neon
        )
    }
}

/// Startup failure when a requested [`SimdPolicy`] cannot run on this build or CPU.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("requested SIMD backend '{requested}' is unavailable on this host ({capabilities})")]
pub struct UnsupportedBackendError {
    requested: SimdPolicy,
    capabilities: CpuCapabilities,
}

impl UnsupportedBackendError {
    /// The policy that could not be satisfied at startup.
    #[must_use]
    pub const fn requested(self) -> SimdPolicy {
        self.requested
    }

    /// The probed host capabilities used to reject the request.
    #[must_use]
    pub const fn capabilities(self) -> CpuCapabilities {
        self.capabilities
    }
}

/// Sealed compile-time SIMD backend token contract.
pub trait SimdBackend: sealed::Sealed + Copy + Send + Sync + 'static {
    /// Underlying `fearless_simd` token type carried by value.
    type Token: Simd;

    /// Observation tag for this backend type.
    const KIND: SelectedBackend;

    /// Lowercase name matching [`SelectedBackend::as_str`].
    const NAME: &'static str;

    /// Returns the genuine `fearless_simd` capability token.
    fn token(self) -> Self::Token;

    /// Executes `f` within the target-feature scope guaranteed by this token.
    fn vectorize<R>(self, f: impl FnOnce() -> R) -> R;
}

/// Sealed x86-64 AVX2 backend carrying a genuine [`fearless_simd::Avx2`] token.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub struct Avx2Backend {
    token: fearless_simd::Avx2,
}

#[cfg(target_arch = "x86_64")]
impl fmt::Debug for Avx2Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Avx2Backend").finish_non_exhaustive()
    }
}

#[cfg(target_arch = "x86_64")]
impl PartialEq for Avx2Backend {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(target_arch = "x86_64")]
impl Eq for Avx2Backend {}

#[cfg(target_arch = "x86_64")]
impl sealed::Sealed for Avx2Backend {}

#[cfg(target_arch = "x86_64")]
impl SimdBackend for Avx2Backend {
    type Token = fearless_simd::Avx2;
    const KIND: SelectedBackend = SelectedBackend::Avx2;
    const NAME: &'static str = "avx2";

    #[inline]
    fn token(self) -> Self::Token {
        self.token
    }

    #[inline]
    fn vectorize<R>(self, f: impl FnOnce() -> R) -> R {
        self.token.vectorize(f)
    }
}

/// Sealed x86-64 AVX-512 backend carrying a genuine [`fearless_simd::Avx512`] token.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub struct Avx512Backend {
    token: fearless_simd::Avx512,
}

#[cfg(target_arch = "x86_64")]
impl fmt::Debug for Avx512Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Avx512Backend").finish_non_exhaustive()
    }
}

#[cfg(target_arch = "x86_64")]
impl PartialEq for Avx512Backend {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(target_arch = "x86_64")]
impl Eq for Avx512Backend {}

#[cfg(target_arch = "x86_64")]
impl sealed::Sealed for Avx512Backend {}

#[cfg(target_arch = "x86_64")]
impl SimdBackend for Avx512Backend {
    type Token = fearless_simd::Avx512;
    const KIND: SelectedBackend = SelectedBackend::Avx512;
    const NAME: &'static str = "avx512";

    #[inline]
    fn token(self) -> Self::Token {
        self.token
    }

    #[inline]
    fn vectorize<R>(self, f: impl FnOnce() -> R) -> R {
        self.token.vectorize(f)
    }
}

/// Sealed AArch64 NEON backend carrying a genuine [`fearless_simd::Neon`] token.
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub struct NeonBackend {
    token: fearless_simd::Neon,
}

#[cfg(target_arch = "aarch64")]
impl fmt::Debug for NeonBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeonBackend").finish_non_exhaustive()
    }
}

#[cfg(target_arch = "aarch64")]
impl PartialEq for NeonBackend {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(target_arch = "aarch64")]
impl Eq for NeonBackend {}

#[cfg(target_arch = "aarch64")]
impl sealed::Sealed for NeonBackend {}

#[cfg(target_arch = "aarch64")]
impl SimdBackend for NeonBackend {
    type Token = fearless_simd::Neon;
    const KIND: SelectedBackend = SelectedBackend::Neon;
    const NAME: &'static str = "neon";

    #[inline]
    fn token(self) -> Self::Token {
        self.token
    }

    #[inline]
    fn vectorize<R>(self, f: impl FnOnce() -> R) -> R {
        self.token.vectorize(f)
    }
}

/// Sealed fallback backend carrying a genuine [`fearless_simd::Fallback`] token.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
#[derive(Clone, Copy)]
pub struct FallbackBackend {
    token: fearless_simd::Fallback,
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl fmt::Debug for FallbackBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FallbackBackend").finish_non_exhaustive()
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl PartialEq for FallbackBackend {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl Eq for FallbackBackend {}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl sealed::Sealed for FallbackBackend {}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
impl SimdBackend for FallbackBackend {
    type Token = fearless_simd::Fallback;
    const KIND: SelectedBackend = SelectedBackend::Fallback;
    const NAME: &'static str = "fallback";

    #[inline]
    fn token(self) -> Self::Token {
        self.token
    }

    #[inline]
    fn vectorize<R>(self, f: impl FnOnce() -> R) -> R {
        self.token.vectorize(f)
    }
}

/// Validated concrete backend produced at session startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResolvedBackend {
    /// x86-64 AVX2 backend token.
    #[cfg(target_arch = "x86_64")]
    Avx2(Avx2Backend),
    /// x86-64 AVX-512 backend token.
    #[cfg(target_arch = "x86_64")]
    Avx512(Avx512Backend),
    /// AArch64 NEON backend token.
    #[cfg(target_arch = "aarch64")]
    Neon(NeonBackend),
    /// Non-x86/non-AArch64 fallback backend token.
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    Fallback(FallbackBackend),
}

/// Typed continuation entered once from a validated [`ResolvedBackend`].
pub trait SimdScope<R> {
    /// Executes with concrete compile-time backend `B` selected at startup.
    fn run<B: SimdBackend>(self, backend: B) -> R;
}

impl ResolvedBackend {
    /// Returns the coarse backend tag for observation outside hot loops.
    #[must_use]
    pub const fn kind(self) -> SelectedBackend {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Avx2(_) => SelectedBackend::Avx2,
            #[cfg(target_arch = "x86_64")]
            Self::Avx512(_) => SelectedBackend::Avx512,
            #[cfg(target_arch = "aarch64")]
            Self::Neon(_) => SelectedBackend::Neon,
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            Self::Fallback(_) => SelectedBackend::Fallback,
        }
    }

    /// Lowercase name matching [`SelectedBackend::as_str`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.kind().as_str()
    }

    /// Enters `scope` with the concrete [`SimdBackend`] represented by `self`.
    #[inline]
    pub fn enter<R>(self, scope: impl SimdScope<R>) -> R {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Avx2(backend) => scope.run(backend),
            #[cfg(target_arch = "x86_64")]
            Self::Avx512(backend) => scope.run(backend),
            #[cfg(target_arch = "aarch64")]
            Self::Neon(backend) => scope.run(backend),
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            Self::Fallback(backend) => scope.run(backend),
        }
    }
}

static CACHED_LEVEL: OnceLock<Level> = OnceLock::new();

fn cached_level() -> Level {
    *CACHED_LEVEL.get_or_init(Level::new)
}

fn capabilities_from_level(level: Level) -> CpuCapabilities {
    #[cfg(target_arch = "x86_64")]
    {
        CpuCapabilities {
            avx2: level.as_avx2().is_some(),
            avx512: level.as_avx512().is_some(),
            neon: false,
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        CpuCapabilities {
            avx2: false,
            avx512: false,
            neon: level.as_neon().is_some(),
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = level;
        CpuCapabilities {
            avx2: false,
            avx512: false,
            neon: false,
        }
    }
}

/// Returns the process-cached host SIMD capability set.
#[must_use]
pub fn detect_capabilities() -> CpuCapabilities {
    capabilities_from_level(cached_level())
}

fn select_policy_kind(
    policy: SimdPolicy,
    caps: CpuCapabilities,
) -> Result<SelectedBackend, UnsupportedBackendError> {
    let err = || UnsupportedBackendError {
        requested: policy,
        capabilities: caps,
    };

    #[cfg(target_arch = "x86_64")]
    {
        match policy {
            SimdPolicy::Auto => {
                if caps.avx512_qualified_for_auto() && caps.avx512 {
                    Ok(SelectedBackend::Avx512)
                } else if caps.avx2 {
                    Ok(SelectedBackend::Avx2)
                } else {
                    Err(err())
                }
            }
            SimdPolicy::Avx2 => caps.avx2.then_some(SelectedBackend::Avx2).ok_or_else(err),
            SimdPolicy::Avx512 => caps
                .avx512
                .then_some(SelectedBackend::Avx512)
                .ok_or_else(err),
            SimdPolicy::Neon => Err(err()),
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        match policy {
            SimdPolicy::Auto | SimdPolicy::Neon => {
                caps.neon.then_some(SelectedBackend::Neon).ok_or_else(err)
            }
            SimdPolicy::Avx2 | SimdPolicy::Avx512 => Err(err()),
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        match policy {
            SimdPolicy::Auto => Ok(SelectedBackend::Fallback),
            SimdPolicy::Avx2 | SimdPolicy::Avx512 | SimdPolicy::Neon => Err(err()),
        }
    }
}

/// Validates and resolves `policy` against the cached host SIMD capabilities.
///
/// # Errors
/// Returns [`UnsupportedBackendError`] when the host CPU or this target build
/// cannot satisfy `policy` cleanly.
pub fn resolve_backend(policy: SimdPolicy) -> Result<ResolvedBackend, UnsupportedBackendError> {
    let level = cached_level();
    let capabilities = capabilities_from_level(level);
    let selected = select_policy_kind(policy, capabilities)?;
    let err = || UnsupportedBackendError {
        requested: policy,
        capabilities,
    };

    #[cfg(target_arch = "x86_64")]
    match selected {
        SelectedBackend::Avx2 => level
            .as_avx2()
            .map(|token| ResolvedBackend::Avx2(Avx2Backend { token }))
            .ok_or_else(err),
        SelectedBackend::Avx512 => level
            .as_avx512()
            .map(|token| ResolvedBackend::Avx512(Avx512Backend { token }))
            .ok_or_else(err),
        SelectedBackend::Neon | SelectedBackend::Fallback => Err(err()),
    }
    #[cfg(target_arch = "aarch64")]
    match selected {
        SelectedBackend::Neon => level
            .as_neon()
            .map(|token| ResolvedBackend::Neon(NeonBackend { token }))
            .ok_or_else(err),
        SelectedBackend::Avx2 | SelectedBackend::Avx512 | SelectedBackend::Fallback => Err(err()),
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    match selected {
        SelectedBackend::Fallback => {
            let token = match level {
                Level::Fallback(token) => token,
                _ => fearless_simd::Fallback::new(),
            };
            Ok(ResolvedBackend::Fallback(FallbackBackend { token }))
        }
        SelectedBackend::Avx2 | SelectedBackend::Avx512 | SelectedBackend::Neon => Err(err()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_detection_is_cached_and_consistent() {
        let first = detect_capabilities();
        let second = detect_capabilities();
        assert_eq!(first, second);
        assert!(!first.avx512_qualified_for_auto());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn policy_matrix_keeps_auto_on_avx2_and_validates_explicit_policies() {
        let avx2_only = CpuCapabilities::from_flags(true, false, false);
        assert_eq!(
            select_policy_kind(SimdPolicy::Auto, avx2_only),
            Ok(SelectedBackend::Avx2)
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Avx2, avx2_only),
            Ok(SelectedBackend::Avx2)
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Avx512, avx2_only),
            Err(UnsupportedBackendError {
                requested: SimdPolicy::Avx512,
                capabilities: avx2_only,
            })
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Neon, avx2_only),
            Err(UnsupportedBackendError {
                requested: SimdPolicy::Neon,
                capabilities: avx2_only,
            })
        );

        let dual = CpuCapabilities::from_flags(true, true, false);
        assert_eq!(
            select_policy_kind(SimdPolicy::Auto, dual),
            Ok(SelectedBackend::Avx2),
            "Auto must not promote unqualified AVX-512 yet"
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Avx512, dual),
            Ok(SelectedBackend::Avx512)
        );

        let none = CpuCapabilities::from_flags(false, false, false);
        for policy in [SimdPolicy::Auto, SimdPolicy::Avx2, SimdPolicy::Avx512] {
            assert!(select_policy_kind(policy, none).is_err());
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn policy_matrix_selects_neon_and_rejects_x86_on_aarch64() {
        let neon = CpuCapabilities::from_flags(false, false, true);
        assert_eq!(
            select_policy_kind(SimdPolicy::Auto, neon),
            Ok(SelectedBackend::Neon)
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Neon, neon),
            Ok(SelectedBackend::Neon)
        );
        assert!(select_policy_kind(SimdPolicy::Avx2, neon).is_err());
        assert!(select_policy_kind(SimdPolicy::Avx512, neon).is_err());

        let none = CpuCapabilities::from_flags(false, false, false);
        assert!(select_policy_kind(SimdPolicy::Auto, none).is_err());
        assert!(select_policy_kind(SimdPolicy::Neon, none).is_err());
    }

    #[test]
    fn resolve_backend_matches_detected_host_capabilities() {
        let caps = detect_capabilities();
        let auto = resolve_backend(SimdPolicy::Auto).expect("supported host baseline");
        #[cfg(target_arch = "x86_64")]
        {
            assert!(caps.has_avx2());
            assert_eq!(auto.kind(), SelectedBackend::Avx2);
            assert_eq!(auto.name(), "avx2");
            assert_eq!(
                resolve_backend(SimdPolicy::Avx2).map(ResolvedBackend::kind),
                Ok(SelectedBackend::Avx2)
            );
            let avx512 = resolve_backend(SimdPolicy::Avx512);
            if caps.has_avx512() {
                assert_eq!(
                    avx512.map(ResolvedBackend::kind),
                    Ok(SelectedBackend::Avx512)
                );
            } else {
                assert_eq!(
                    avx512,
                    Err(UnsupportedBackendError {
                        requested: SimdPolicy::Avx512,
                        capabilities: caps,
                    })
                );
            }
            assert!(resolve_backend(SimdPolicy::Neon).is_err());
        }
        #[cfg(target_arch = "aarch64")]
        {
            assert!(caps.has_neon());
            assert_eq!(auto.kind(), SelectedBackend::Neon);
            assert_eq!(auto.name(), "neon");
            assert_eq!(
                resolve_backend(SimdPolicy::Neon).map(ResolvedBackend::kind),
                Ok(SelectedBackend::Neon)
            );
            assert!(resolve_backend(SimdPolicy::Avx2).is_err());
            assert!(resolve_backend(SimdPolicy::Avx512).is_err());
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = caps;
            assert_eq!(auto.kind(), SelectedBackend::Fallback);
        }
    }

    #[test]
    fn resolved_backend_enter_dispatches_concrete_backend_kind() {
        struct KindProbe;

        impl SimdScope<SelectedBackend> for KindProbe {
            fn run<B: SimdBackend>(self, _backend: B) -> SelectedBackend {
                B::KIND
            }
        }

        let resolved = resolve_backend(SimdPolicy::Auto).expect("supported host baseline");
        assert_eq!(resolved.enter(KindProbe), resolved.kind());
    }
}
