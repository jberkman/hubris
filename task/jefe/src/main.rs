// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Implementation of the system supervisor.
//!
//! The supervisor is responsible for:
//!
//! - Maintaining the system console output (currently via semihosting).
//! - Monitoring tasks for failures and restarting them.
//!
//! It will probably become responsible for:
//!
//! - Evacuating kernel log information.
//! - Coordinating certain shared resources, such as the RCC and GPIO muxing.
//! - Managing a watchdog timer.
//!
//! It's unwise for the supervisor to use `SEND`, ever, except to talk to the
//! kernel. This is because a `SEND` to a misbehaving task could block forever,
//! taking out the supervisor. The long-term idea is to provide some sort of
//! asynchronous messaging from the supervisor to less-trusted tasks, but that
//! doesn't exist yet, so we're mostly using RECV/REPLY and notifications. This
//! means that hardware drivers required for this task must be built in instead
//! of running in separate tasks.

#![no_std]
#![no_main]

mod external;

use core::convert::Infallible;

use hubris_num_tasks::NUM_TASKS;
use task_jefe_api::ResetReason;
use userlib::*;

fn log_fault(t: usize, fault: &abi::FaultInfo) {
    match fault {
        abi::FaultInfo::MemoryAccess { address, .. } => match address {
            Some(a) => {
                sys_log!("Task #{} Memory fault at address {:#x}", t, a);
            }

            None => {
                sys_log!("Task #{} Memory fault at unknown address", t);
            }
        },

        abi::FaultInfo::BusError { address, .. } => match address {
            Some(a) => {
                sys_log!("Task #{} Bus error at address {:#x}", t, a);
            }

            None => {
                sys_log!("Task #{} Bus error at unknown address", t);
            }
        },

        abi::FaultInfo::StackOverflow { address, .. } => {
            sys_log!("Task #{} Stack overflow at address {:#x}", t, address);
        }

        abi::FaultInfo::DivideByZero => {
            sys_log!("Task #{} Divide-by-zero", t);
        }

        abi::FaultInfo::IllegalText => {
            sys_log!("Task #{} Illegal text", t);
        }

        abi::FaultInfo::IllegalInstruction => {
            sys_log!("Task #{} Illegal instruction", t);
        }

        abi::FaultInfo::InvalidOperation(details) => {
            sys_log!("Task #{} Invalid operation: {:#010x}", t, details);
        }

        abi::FaultInfo::SyscallUsage(e) => {
            sys_log!("Task #{} Bad Syscall Usage {:?}", t, e);
        }

        abi::FaultInfo::Panic => {
            sys_log!("Task #{} Panic!", t);
        }

        abi::FaultInfo::Injected(who) => {
            sys_log!("Task #{} Fault injected by task #{}", t, who.index());
        }
        abi::FaultInfo::FromServer(who, what) => {
            sys_log!(
                "Task #{} Fault from server #{}: {:?}",
                t,
                who.index(),
                what
            );
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum Disposition {
    #[default]
    Restart,
    Hold,
}

// We install a timeout to periodcally check for an external direction
// of our task disposition (e.g., via Humility).  This timeout should
// generally be fast for a human but slow for a computer; we pick a
// value of ~100 ms.  Our timer mask can't conflict with our fault
// notification, but can otherwise be arbitrary.
const TIMER_INTERVAL: u64 = 100;
const TIMER_MASK: u32 = 1 << 1;
// We'll have notification 0 wired up to receive information about task faults.
const FAULT_MASK: u32 = 1 << 0;

#[export_name = "main"]
fn main() -> ! {
    sys_log!("viva el jefe");

    let mut task_states = [TaskStatus::default(); hubris_num_tasks::NUM_TASKS];
    for held_task in generated::HELD_TASKS {
        task_states[held_task as usize].disposition = Disposition::Hold;
    }

    let deadline = sys_get_timer().now + TIMER_INTERVAL;

    sys_set_timer(Some(deadline), TIMER_MASK);

    external::set_ready();

    let mut server = ServerImpl {
        state: 0,
        deadline,
        task_states: &mut task_states,
        reset_reason: ResetReason::Unknown,
    };
    let mut buf = [0u8; idl::INCOMING_SIZE];

    loop {
        idol_runtime::dispatch_n(&mut buf, &mut server);
    }
}

struct ServerImpl<'s> {
    state: u32,
    task_states: &'s mut [TaskStatus; NUM_TASKS],
    deadline: u64,
    reset_reason: ResetReason,
}

impl idl::InOrderJefeImpl for ServerImpl<'_> {
    fn request_reset(
        &mut self,
        _msg: &userlib::RecvMessage,
    ) -> Result<(), idol_runtime::RequestError<Infallible>> {
        // If we wanted to broadcast to other tasks that a restart is occuring
        // here is where we would do so!
        kipc::system_restart();
    }

    fn get_reset_reason(
        &mut self,
        _msg: &userlib::RecvMessage,
    ) -> Result<ResetReason, idol_runtime::RequestError<Infallible>> {
        Ok(self.reset_reason)
    }

    fn set_reset_reason(
        &mut self,
        _msg: &userlib::RecvMessage,
        reason: ResetReason,
    ) -> Result<(), idol_runtime::RequestError<Infallible>> {
        self.reset_reason = reason;
        Ok(())
    }

    fn get_state(
        &mut self,
        _msg: &userlib::RecvMessage,
    ) -> Result<u32, idol_runtime::RequestError<Infallible>> {
        Ok(self.state)
    }

    fn set_state(
        &mut self,
        _msg: &userlib::RecvMessage,
        state: u32,
    ) -> Result<(), idol_runtime::RequestError<Infallible>> {
        if self.state != state {
            self.state = state;

            for (task, mask) in generated::MAILING_LIST {
                let taskid =
                    TaskId::for_index_and_gen(task as usize, Generation::ZERO);
                let taskid = sys_refresh_task_id(taskid);
                sys_post(taskid, mask);
            }
        }
        Ok(())
    }
}

/// Structure we use for tracking the state of the tasks we supervise. There is
/// one of these per supervised task.
#[derive(Copy, Clone, Debug, Default)]
struct TaskStatus {
    disposition: Disposition,
    holding_fault: bool,
}

impl idol_runtime::NotificationHandler for ServerImpl<'_> {
    fn current_notification_mask(&self) -> u32 {
        FAULT_MASK | TIMER_MASK
    }

    fn handle_notification(&mut self, bits: u32) {
        // Handle any external (debugger) requests.
        external::check(self.task_states);

        if bits & TIMER_MASK != 0 {
            // If our timer went off, we need to reestablish it
            if sys_get_timer().now >= self.deadline {
                self.deadline += TIMER_INTERVAL;
                sys_set_timer(Some(self.deadline), TIMER_MASK);
            }
        }

        if bits & FAULT_MASK != 0 {
            // Work out who faulted. It's theoretically possible for more than
            // one task to have faulted since we last looked, but it's somewhat
            // unlikely since a fault causes us to immediately preempt. In any
            // case, let's assume we might have to handle multiple tasks.
            //
            // TODO: it would be fantastic to have a way of finding this out in
            // one syscall.
            for (i, status) in self.task_states.iter_mut().enumerate() {
                // If we're aware that this task is in a fault state, don't
                // bother making a syscall to enquire.
                if status.holding_fault {
                    continue;
                }

                match kipc::read_task_status(i) {
                    abi::TaskState::Faulted { fault, .. } => {
                        // Well! A fault we didn't know about.
                        log_fault(i, &fault);

                        if status.disposition == Disposition::Restart {
                            // Stand it back up
                            kipc::restart_task(i, true);
                        } else {
                            // Mark this one off so we don't revisit it until
                            // requested.
                            status.holding_fault = true;
                        }
                    }

                    // For the purposes of this loop, ignore all other tasks.
                    _ => (),
                }
            }
        }
    }
}

// Place to namespace all the bits generated by our config processor.
mod generated {
    include!(concat!(env!("OUT_DIR"), "/jefe_config.rs"));
}

// And the Idol bits
mod idl {
    use task_jefe_api::ResetReason;
    include!(concat!(env!("OUT_DIR"), "/server_stub.rs"));
}
