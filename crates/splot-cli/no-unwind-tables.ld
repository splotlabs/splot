/* SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 */
/* SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> */

/* INSERT keeps the linker's default layout; a bare SECTIONS replaces it. */
SECTIONS
{
  /DISCARD/ : { *(.eh_frame) *(.gcc_except_table .gcc_except_table.*) }
}
INSERT AFTER .text;
