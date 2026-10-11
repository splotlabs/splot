/* SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 */
/* SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> */

/*
 * Counts the instructions that the main executable runs, for
 * `cargo xtask perf`. Each basic block of the executable adds its
 * instruction count to one 64-bit counter with an atomic (locked) add, so
 * threads cannot lose updates. Blocks of shared libraries are not counted,
 * because glibc selects its string functions per CPU. At exit the client
 * prints "Instrumentation results: N instructions executed".
 */

#include "dr_api.h"
#include "drmgr.h"
#include "drx.h"

static app_pc exe_start;
/* One cache line, so the locked add never crosses a line. */
static uint64 global_count __attribute__((aligned(64)));

static dr_emit_flags_t
event_bb_analysis(void *drcontext, void *tag, instrlist_t *bb, bool for_trace,
                  bool translating, void **user_data)
{
    uint num_instrs = 0;
    module_data_t *mod = dr_lookup_module(dr_fragment_app_pc(tag));
    bool from_exe = mod == NULL || mod->start == exe_start;
    if (mod != NULL)
        dr_free_module_data(mod);
    if (from_exe) {
        for (instr_t *instr = instrlist_first_app(bb); instr != NULL;
             instr = instr_get_next_app(instr))
            num_instrs++;
    }
    *user_data = (void *)(ptr_uint_t)num_instrs;
    return DR_EMIT_DEFAULT;
}

static dr_emit_flags_t
event_app_instruction(void *drcontext, void *tag, instrlist_t *bb, instr_t *instr,
                      bool for_trace, bool translating, void *user_data)
{
    uint num_instrs = (uint)(ptr_uint_t)user_data;
    drmgr_disable_auto_predication(drcontext, bb);
    if (num_instrs == 0 || !drmgr_is_first_instr(drcontext, instr))
        return DR_EMIT_DEFAULT;
    if (!drx_insert_counter_update(drcontext, bb, instr, SPILL_SLOT_MAX + 1,
                                   &global_count, (int)num_instrs,
                                   DRX_COUNTER_64BIT | DRX_COUNTER_LOCK))
        DR_ASSERT_MSG(false, "atomic counter update failed");
    return DR_EMIT_DEFAULT;
}

static void
event_exit(void)
{
    dr_printf("Instrumentation results: %llu instructions executed\n",
              (unsigned long long)global_count);
    drx_exit();
    drmgr_exit();
}

DR_EXPORT void
dr_client_main(client_id_t id, int argc, const char *argv[])
{
    dr_set_client_name("splot perf instruction counter",
                       "https://github.com/splotlabs/splot");
    if (!drmgr_init() || !drx_init())
        DR_ASSERT(false);
    module_data_t *exe = dr_get_main_module();
    if (exe != NULL) {
        exe_start = exe->start;
        dr_free_module_data(exe);
    }
    dr_register_exit_event(event_exit);
    if (!drmgr_register_bb_instrumentation_event(event_bb_analysis,
                                                 event_app_instruction, NULL))
        DR_ASSERT(false);
}
