// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Process-lifetime cache for `SPLOT_TRACE_*` diagnostic env gates.
//!
//! Hot decode paths consult these gates per block or per symbol; reading the
//! environment on each check takes a process-global lock on some platforms.
//! Each gate is read once per process, matching the variables' contract of
//! selecting diagnostics at launch.

/// Returns the cached presence of a diagnostic env var, reading it once.
macro_rules! trace_flag {
    ($name:literal) => {{
        static FLAG: ::std::sync::OnceLock<bool> = ::std::sync::OnceLock::new();
        *FLAG.get_or_init(|| ::std::env::var_os($name).is_some())
    }};
}

/// Returns the cached UTF-8 value of a diagnostic env var, reading it once.
macro_rules! trace_value {
    ($name:literal) => {{
        static VALUE: ::std::sync::OnceLock<Option<String>> = ::std::sync::OnceLock::new();
        VALUE
            .get_or_init(|| ::std::env::var($name).ok())
            .as_deref()
    }};
}

pub(crate) use {trace_flag, trace_value};
