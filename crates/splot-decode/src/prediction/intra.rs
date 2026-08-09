// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::support::capability::missing_capability_message;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraLumaUnsupported {
    UnsupportedMode,
}

impl IntraLumaUnsupported {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::UnsupportedMode => {
                missing_capability_message!("intra.luma.mode", mode = "unsupported")
            }
        }
    }
}

pub(crate) const UNSUPPORTED_LUMA_MODE: IntraLumaUnsupported =
    IntraLumaUnsupported::UnsupportedMode;
