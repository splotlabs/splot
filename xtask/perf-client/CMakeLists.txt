# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#
# DynamoRIO client for `cargo xtask perf`. Configure with
# -DDynamoRIO_DIR=<DynamoRIO release>/cmake.
cmake_minimum_required(VERSION 3.14)
project(splot_inscount C)
find_package(DynamoRIO REQUIRED)
add_library(splotinscount SHARED splotinscount.c)
configure_DynamoRIO_client(splotinscount)
use_DynamoRIO_extension(splotinscount drmgr)
use_DynamoRIO_extension(splotinscount drreg)
use_DynamoRIO_extension(splotinscount drx)
