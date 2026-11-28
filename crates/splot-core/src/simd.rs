// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The SIMD backend value and startup capability selection.
//!
//! Host capabilities are probed once per process via `fearless_simd` and
//! cached in an immutable `OnceLock`. Callers resolve a requested
//! [`SimdPolicy`] at session initialization into a [`ResolvedBackend`], whose
//! [`SimdBackend`] holds genuine SIMD capability tokens, keeping hot loops
//! free of feature queries. On a build whose floor is the level it runs, the
//! backend is zero-sized; an x86-64 build below the v3 floor matches on SSE2
//! or AVX2 at each kernel entry.

use core::fmt;
use std::sync::OnceLock;

use fearless_simd::{Level, Simd};

#[cfg(target_arch = "x86_64")]
const STATIC_AVX2: bool = matches!(Level::baseline(), Level::Avx2(_) | Level::Avx512(_));

#[cfg(all(test, not(target_arch = "x86_64")))]
const STATIC_AVX2: bool = false;

#[cfg(target_arch = "x86_64")]
mod sse2_floor {
    use core::convert::Infallible;

    pub(super) struct X86Floor<const HAS_AVX2: bool>;

    pub(super) trait Sse2Support {
        type Proof: Copy;
        const AVAILABLE: Option<Self::Proof>;
    }

    impl Sse2Support for X86Floor<false> {
        type Proof = ();
        const AVAILABLE: Option<Self::Proof> = Some(());
    }

    impl Sse2Support for X86Floor<true> {
        type Proof = Infallible;
        const AVAILABLE: Option<Self::Proof> = None;
    }

    pub(super) type BuildFloor = X86Floor<{ super::STATIC_AVX2 }>;
}

/// Requested SIMD execution policy resolved at initialization time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SimdPolicy {
    /// Selects the crate's verified default execution backend for this host.
    #[default]
    Auto,
    /// Requires the SSE2 backend provided by x86-64 builds below the full v3 floor.
    Sse2,
    /// Requires the full x86-64-v3 SIMD feature and OS-state bundle.
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
            Self::Sse2 => "sse2",
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
    /// x86-64 SSE2 backend.
    Sse2,
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
            Self::Sse2 => "sse2",
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
    sse2: bool,
    avx2: bool,
    avx512: bool,
    neon: bool,
}

impl CpuCapabilities {
    /// Whether an x86-64 SSE2 backend is available.
    #[must_use]
    pub const fn has_sse2(self) -> bool {
        self.sse2
    }

    /// Whether the host CPU and OS pass `fearless_simd`'s x86-64-v3 check.
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
        self.avx512 && self.avx2
    }

    #[cfg(test)]
    const fn from_flags(sse2: bool, avx2: bool, avx512: bool, neon: bool) -> Self {
        Self {
            sse2: sse2 && !STATIC_AVX2,
            avx2,
            avx512,
            neon,
        }
    }
}

impl fmt::Display for CpuCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sse2={}, avx2={}, avx512={}, neon={}",
            self.sse2, self.avx2, self.avx512, self.neon
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

/// SIMD level of an x86-64 decoder, for [`with_simd!`].
#[cfg(target_arch = "x86_64")]
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum X86Simd {
    /// The full x86-64-v3 bundle.
    Avx2(fearless_simd::Avx2),
    /// The x86-64 baseline of a build below the v3 floor.
    Sse2(Sse2Floor),
}

/// SSE2 token of a build below the full v3 floor. A v3 build cannot make one,
/// which removes the SSE2 kernels from it.
#[cfg(target_arch = "x86_64")]
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct Sse2Floor(
    (
        fearless_simd::Sse2,
        <sse2_floor::BuildFloor as sse2_floor::Sse2Support>::Proof,
    ),
);

#[cfg(target_arch = "x86_64")]
impl Sse2Floor {
    /// The genuine `fearless_simd` SSE2 token.
    #[inline]
    #[must_use]
    pub fn token(self) -> fearless_simd::Sse2 {
        self.0.0
    }
}

#[cfg(target_arch = "x86_64")]
type Tokens = X86Simd;
#[cfg(target_arch = "aarch64")]
type Tokens = fearless_simd::Neon;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
type Tokens = fearless_simd::Fallback;

/// The SIMD level a decoder's kernels run at, selected once at startup and
/// carried by value: genuine `fearless_simd` capability tokens.
///
/// The decode pipeline is compiled once and hands this to its kernels. It is
/// zero-sized where the build floor leaves one level to run: x86-64-v3,
/// AArch64 and the fallback targets. An x86-64 build below the v3 floor holds
/// SSE2 or AVX2 here. AVX-512 is not a level of this type: the kernels that
/// have a wider form reach it through [`ResolvedBackend::avx512_token`].
#[derive(Clone, Copy)]
pub struct SimdBackend(Tokens);

impl SimdBackend {
    /// Observation tag of the level this backend runs.
    #[must_use]
    pub const fn kind(self) -> SelectedBackend {
        #[cfg(target_arch = "x86_64")]
        match self.0 {
            X86Simd::Avx2(_) => SelectedBackend::Avx2,
            X86Simd::Sse2(_) => SelectedBackend::Sse2,
        }
        #[cfg(target_arch = "aarch64")]
        {
            SelectedBackend::Neon
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            SelectedBackend::Fallback
        }
    }

    /// Lowercase name matching [`SelectedBackend::as_str`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.kind().as_str()
    }

    /// Executes `f` within the target-feature scope this backend guarantees.
    #[inline]
    pub fn vectorize<R>(self, f: impl FnOnce() -> R) -> R {
        #[cfg(target_arch = "x86_64")]
        match self.0 {
            X86Simd::Avx2(token) => token.vectorize(f),
            X86Simd::Sse2(floor) => floor.token().vectorize(f),
        }
        #[cfg(not(target_arch = "x86_64"))]
        self.0.vectorize(f)
    }

    /// The `Avx2` token of an x86-64 backend that runs AVX2.
    #[cfg(target_arch = "x86_64")]
    #[inline]
    #[must_use]
    pub fn avx2_token(self) -> Option<fearless_simd::Avx2> {
        match self.0 {
            X86Simd::Avx2(token) => Some(token),
            X86Simd::Sse2(_) => None,
        }
    }

    /// The `Neon` token of an AArch64 backend.
    #[cfg(target_arch = "aarch64")]
    #[inline]
    #[must_use]
    pub fn neon_token(self) -> Option<fearless_simd::Neon> {
        Some(self.0)
    }

    /// The tokens [`with_simd!`] matches on.
    #[cfg(target_arch = "x86_64")]
    #[doc(hidden)]
    #[inline]
    #[must_use]
    pub fn x86_simd(self) -> X86Simd {
        self.0
    }

    /// The token [`with_simd!`] runs its body with.
    #[cfg(not(target_arch = "x86_64"))]
    #[doc(hidden)]
    #[inline]
    #[must_use]
    pub fn token(self) -> Tokens {
        self.0
    }
}

impl fmt::Debug for SimdBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SimdBackend").field(&self.kind()).finish()
    }
}

impl PartialEq for SimdBackend {
    fn eq(&self, other: &Self) -> bool {
        self.kind() == other.kind()
    }
}

impl Eq for SimdBackend {}

/// [`fearless_simd::Simd::vectorize`] for [`with_simd!`], which expands in
/// crates that do not name the trait.
#[doc(hidden)]
#[inline]
pub fn vectorize_token<S: Simd, R>(simd: S, f: impl FnOnce() -> R) -> R {
    simd.vectorize(f)
}

/// Runs `$body` with the concrete `fearless_simd` token `$simd` of
/// `$backend`. Where static target features already guarantee the token's
/// instruction set, the body runs in place; otherwise it runs in the token's
/// target-feature scope.
#[macro_export]
macro_rules! with_simd {
    ($backend:expr, $simd:ident => $body:expr) => {{
        let __backend: $crate::simd::SimdBackend = $backend;
        #[cfg(target_arch = "x86_64")]
        {
            match $crate::simd::SimdBackend::x86_simd(__backend) {
                $crate::simd::X86Simd::Avx2($simd) => {
                    #[cfg(all(
                        target_feature = "avx2",
                        target_feature = "fma",
                        target_feature = "bmi1",
                        target_feature = "bmi2",
                        target_feature = "f16c",
                        target_feature = "lzcnt"
                    ))]
                    {
                        $body
                    }
                    #[cfg(not(all(
                        target_feature = "avx2",
                        target_feature = "fma",
                        target_feature = "bmi1",
                        target_feature = "bmi2",
                        target_feature = "f16c",
                        target_feature = "lzcnt"
                    )))]
                    {
                        $crate::simd::vectorize_token($simd, || $body)
                    }
                }
                $crate::simd::X86Simd::Sse2(__floor) => {
                    let $simd = __floor.token();
                    $crate::simd::vectorize_token($simd, || $body)
                }
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            let $simd = $crate::simd::SimdBackend::token(__backend);
            #[cfg(target_feature = "neon")]
            {
                $body
            }
            #[cfg(not(target_feature = "neon"))]
            {
                $crate::simd::vectorize_token($simd, || $body)
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let $simd = $crate::simd::SimdBackend::token(__backend);
            $crate::simd::vectorize_token($simd, || $body)
        }
    }};
}

/// Validated backend produced at session startup: the level the kernels run
/// at, and the AVX-512 token when the policy selected AVX-512.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedBackend {
    backend: SimdBackend,
    wide: bool,
}

impl ResolvedBackend {
    /// Returns the coarse backend tag for observation outside hot loops.
    #[must_use]
    pub const fn kind(self) -> SelectedBackend {
        if self.wide {
            SelectedBackend::Avx512
        } else {
            self.backend.kind()
        }
    }

    /// Lowercase name matching [`SelectedBackend::as_str`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.kind().as_str()
    }

    /// The level the decode pipeline's kernels run at. An AVX-512 selection
    /// runs AVX2 here and reaches its wider kernels through
    /// [`Self::avx512_token`].
    #[must_use]
    pub const fn backend(self) -> SimdBackend {
        self.backend
    }

    /// The `Avx512` token of an AVX-512 selection, for the kernels that have
    /// a wider form than [`Self::backend`] runs.
    #[cfg(target_arch = "x86_64")]
    #[must_use]
    pub fn avx512_token(self) -> Option<fearless_simd::Avx512> {
        if self.wide {
            cached_level().as_avx512()
        } else {
            None
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
            sse2: !STATIC_AVX2 && level.as_sse2().is_some(),
            avx2: level.as_avx2().is_some(),
            avx512: level.as_avx512().is_some(),
            neon: false,
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        CpuCapabilities {
            sse2: false,
            avx2: false,
            avx512: false,
            neon: level.as_neon().is_some(),
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = level;
        CpuCapabilities {
            sse2: false,
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
                if caps.avx512_qualified_for_auto() {
                    Ok(SelectedBackend::Avx512)
                } else if caps.avx2 {
                    Ok(SelectedBackend::Avx2)
                } else {
                    caps.has_sse2()
                        .then_some(SelectedBackend::Sse2)
                        .ok_or_else(err)
                }
            }
            SimdPolicy::Sse2 => caps
                .has_sse2()
                .then_some(SelectedBackend::Sse2)
                .ok_or_else(err),
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
            SimdPolicy::Sse2 | SimdPolicy::Avx2 | SimdPolicy::Avx512 => Err(err()),
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        match policy {
            SimdPolicy::Auto => Ok(SelectedBackend::Fallback),
            SimdPolicy::Sse2 | SimdPolicy::Avx2 | SimdPolicy::Avx512 | SimdPolicy::Neon => {
                Err(err())
            }
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
        SelectedBackend::Avx2 | SelectedBackend::Avx512 => level
            .as_avx2()
            .filter(|_| selected == SelectedBackend::Avx2 || level.as_avx512().is_some())
            .map(|token| ResolvedBackend {
                backend: SimdBackend(X86Simd::Avx2(token)),
                wide: selected == SelectedBackend::Avx512,
            })
            .ok_or_else(err),
        SelectedBackend::Sse2 => {
            use sse2_floor::Sse2Support as _;
            level
                .as_sse2()
                .zip(sse2_floor::BuildFloor::AVAILABLE)
                .map(|tokens| ResolvedBackend {
                    backend: SimdBackend(X86Simd::Sse2(Sse2Floor(tokens))),
                    wide: false,
                })
                .ok_or_else(err)
        }
        SelectedBackend::Neon | SelectedBackend::Fallback => Err(err()),
    }
    #[cfg(target_arch = "aarch64")]
    match selected {
        SelectedBackend::Neon => level
            .as_neon()
            .map(|token| ResolvedBackend {
                backend: SimdBackend(token),
                wide: false,
            })
            .ok_or_else(err),
        SelectedBackend::Sse2
        | SelectedBackend::Avx2
        | SelectedBackend::Avx512
        | SelectedBackend::Fallback => Err(err()),
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    match selected {
        SelectedBackend::Fallback => {
            let token = match level {
                Level::Fallback(token) => token,
                _ => fearless_simd::Fallback::new(),
            };
            Ok(ResolvedBackend {
                backend: SimdBackend(token),
                wide: false,
            })
        }
        SelectedBackend::Sse2
        | SelectedBackend::Avx2
        | SelectedBackend::Avx512
        | SelectedBackend::Neon => Err(err()),
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
        assert_eq!(
            first.avx512_qualified_for_auto(),
            first.has_avx512() && first.has_avx2()
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn policy_matrix_selects_the_best_available_x86_backend() {
        let avx2_only = CpuCapabilities::from_flags(true, true, false, false);
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

        let dual = CpuCapabilities::from_flags(true, true, true, false);
        assert_eq!(
            select_policy_kind(SimdPolicy::Auto, dual),
            Ok(SelectedBackend::Avx512)
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Avx512, dual),
            Ok(SelectedBackend::Avx512)
        );

        let sse2_only = CpuCapabilities::from_flags(true, false, false, false);
        for policy in [SimdPolicy::Auto, SimdPolicy::Sse2] {
            assert_eq!(
                select_policy_kind(policy, sse2_only).ok(),
                sse2_only.has_sse2().then_some(SelectedBackend::Sse2)
            );
        }
        for policy in [SimdPolicy::Avx2, SimdPolicy::Avx512, SimdPolicy::Neon] {
            assert!(select_policy_kind(policy, sse2_only).is_err());
        }

        let none = CpuCapabilities::from_flags(false, false, false, false);
        for policy in [
            SimdPolicy::Auto,
            SimdPolicy::Sse2,
            SimdPolicy::Avx2,
            SimdPolicy::Avx512,
        ] {
            assert!(select_policy_kind(policy, none).is_err());
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn policy_matrix_selects_neon_and_rejects_x86_on_aarch64() {
        let neon = CpuCapabilities::from_flags(false, false, false, true);
        assert_eq!(
            select_policy_kind(SimdPolicy::Auto, neon),
            Ok(SelectedBackend::Neon)
        );
        assert_eq!(
            select_policy_kind(SimdPolicy::Neon, neon),
            Ok(SelectedBackend::Neon)
        );
        assert!(select_policy_kind(SimdPolicy::Sse2, neon).is_err());
        assert!(select_policy_kind(SimdPolicy::Avx2, neon).is_err());
        assert!(select_policy_kind(SimdPolicy::Avx512, neon).is_err());

        let none = CpuCapabilities::from_flags(false, false, false, false);
        assert!(select_policy_kind(SimdPolicy::Auto, none).is_err());
        assert!(select_policy_kind(SimdPolicy::Neon, none).is_err());
    }

    #[test]
    fn resolve_backend_matches_detected_host_capabilities() {
        let caps = detect_capabilities();
        let auto = resolve_backend(SimdPolicy::Auto).expect("supported host baseline");
        #[cfg(target_arch = "x86_64")]
        {
            assert_eq!(caps.has_sse2(), Level::baseline().as_avx2().is_none());
            assert_eq!(
                resolve_backend(SimdPolicy::Sse2)
                    .map(ResolvedBackend::kind)
                    .ok(),
                caps.has_sse2().then_some(SelectedBackend::Sse2)
            );
            let expected = if caps.has_avx512() {
                SelectedBackend::Avx512
            } else if caps.has_avx2() {
                SelectedBackend::Avx2
            } else {
                SelectedBackend::Sse2
            };
            assert_eq!(auto.kind(), expected);
            assert_eq!(resolve_backend(SimdPolicy::Avx2).is_ok(), caps.has_avx2());
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
            assert!(resolve_backend(SimdPolicy::Sse2).is_err());
            assert!(resolve_backend(SimdPolicy::Avx2).is_err());
            assert!(resolve_backend(SimdPolicy::Avx512).is_err());
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = caps;
            assert_eq!(auto.kind(), SelectedBackend::Fallback);
            assert!(resolve_backend(SimdPolicy::Sse2).is_err());
        }
    }

    #[test]
    fn the_backend_runs_the_level_below_a_wide_selection() {
        let resolved = resolve_backend(SimdPolicy::Auto).expect("supported host baseline");
        let backend = resolved.backend();
        let expected = match resolved.kind() {
            SelectedBackend::Avx512 => SelectedBackend::Avx2,
            other => other,
        };
        assert_eq!(backend.kind(), expected);
        assert_eq!(backend.vectorize(|| backend.name()), expected.as_str());
        #[cfg(target_arch = "x86_64")]
        {
            assert_eq!(
                backend.avx2_token().is_some(),
                backend.kind() != SelectedBackend::Sse2
            );
            assert_eq!(
                resolved.avx512_token().is_some(),
                resolved.kind() == SelectedBackend::Avx512
            );
            if STATIC_AVX2 {
                assert_eq!(core::mem::size_of::<SimdBackend>(), 0);
            }
        }
        #[cfg(target_arch = "aarch64")]
        assert!(backend.neon_token().is_some());
    }
}
