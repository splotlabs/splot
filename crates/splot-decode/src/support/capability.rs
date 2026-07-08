// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

macro_rules! missing_capability_message {
    ($id:literal $(, $key:ident = $value:literal)* $(,)?) => {
        concat!("unsupported capability: ", $id $(, " ", stringify!($key), "=", $value)*)
    };
}

pub(crate) use missing_capability_message;

#[cfg(test)]
#[path = "capability_tests.rs"]
mod tests;
