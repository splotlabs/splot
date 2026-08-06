// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::support::capability::missing_capability_message;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraLumaUnsupported {
    reason_id: &'static str,
    message: &'static str,
}

impl IntraLumaUnsupported {
    pub(crate) const fn reason_id(self) -> &'static str {
        self.reason_id
    }

    pub(crate) const fn message(self) -> &'static str {
        self.message
    }
}

const fn unsupported(reason_id: &'static str, message: &'static str) -> IntraLumaUnsupported {
    IntraLumaUnsupported { reason_id, message }
}

pub(crate) const UNSUPPORTED_LUMA_MODE: IntraLumaUnsupported = unsupported(
    "general_intra_unsupported_luma_mode",
    missing_capability_message!("intra.luma.mode", mode = "unsupported"),
);
