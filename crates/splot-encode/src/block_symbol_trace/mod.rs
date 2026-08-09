// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Entropy coding for the supported general-intra block trace.

use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_LF_CDF, DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
    DEFAULT_COEFF_BR_LF_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_EOB_EXTRA_CDF,
    DEFAULT_EOB_PT_1024_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use crate::coefficient_tokenization::{CoefficientCdfRowSelector, CoefficientEntropyToken};
use crate::error::{Error, Result};
use crate::intra_mode_emission::{IntraModeCdfRowSelector, IntraModeToken};
use crate::partition_emission::{
    PartitionToken, ROOT_64X64_DO_SPLIT_CTX, ROOT_PARTITION_PLANE_START,
};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const TXB_SKIP_CDF_ROW_LEN: usize = 3;
const DO_SPLIT_CDF_ROW_LEN: usize = 3;
const V_TXB_SKIP_CDF_ROW_LEN: usize = 3;
const EOB_PT_1024_CDF_ROW_LEN: usize = 9;
const EOB_EXTRA_CDF_ROW_LEN: usize = 3;
const COEFF_BASE_LF_EOB_CDF_ROW_LEN: usize = 6;
const COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN: usize = 6;
const COEFF_BR_LF_CDF_ROW_LEN: usize = 5;
const COEFF_BASE_LF_CDF_ROW_LEN: usize = 7;
const DC_SIGN_CDF_ROW_LEN: usize = 3;
const COEFF_CDF_Q_CTX: usize = 0;
const LUMA_PLANE_TYPE: usize = 0;
const TX_SIZE_64X64_CTX: usize = 4;
const TX_SIZE_32X32_CTX: usize = 3;
const TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_U_TXB_SKIP_CTX_NEUTRAL: usize = 6;
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
const CHROMA_V_TXB_SKIP_CTX_EOBU: usize = 6;
const EOB_CTX_LUMA_INTRA: usize = 0;
const EOB_CTX_CHROMA: usize = 2;
const COEFF_BASE_LF_EOB_CTX_DC: usize = 0;
const COEFF_BASE_LF_EOB_CTX_AC: usize = 1;
const COEFF_BASE_LF_CTX_EOB2_DC: usize = 1;
const COEFF_BASE_LF_CTX_VISIBLE_AC_DC: usize = 2;
const COEFF_BASE_LF_CTX_AC_BAND: usize = 9;
const COEFF_BASE_LF_CTX_2D_DC: usize = 4;
const COEFF_BASE_LF_TCQ_CTX: usize = 0;
const COEFF_BR_LF_CTX_DC: usize = 0;
const DC_SIGN_GROUP_VISIBLE: usize = 0;
const DC_SIGN_CTX_NEUTRAL: usize = 0;
const BLOCK_SYMBOL_TRACE_BUDGET_HEADROOM: usize = 32;

mod cdf_rows;
mod coder;
mod compose;

use cdf_rows::BlockSymbolTraceCdfRows;

pub(crate) use coder::{BlockSymbolToken, encode_block_symbol_trace};
pub(crate) use compose::compose_minimal_intra_dc_block_mode_trace;
