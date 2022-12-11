// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![feature(cmse_nonsecure_entry)]
#![feature(naked_functions)]
#![feature(array_methods)]
#![no_main]
#![no_std]

use core::arch;

extern crate lpc55_pac;
extern crate panic_halt;
use cortex_m::peripheral::Peripherals;
use cortex_m_rt::entry;

cfg_if::cfg_if! {
    if #[cfg(feature = "dice-mfg")] {
        mod dice;
        mod dice_mfg_usart;
    } else if #[cfg(feature = "dice-self")] {
        mod dice;
        mod dice_mfg_self;
    }
}

// FIXME Need to fixup the secure interface calls
//mod hypo;
mod image_header;

use crate::image_header::Image;

/// Initial entry point for handling a memory management fault.
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn MemoryManagement() {
    loop {}
}

/// Initial entry point for handling a bus fault.
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn BusFault() {
    loop {}
}

/// Initial entry point for handling a usage fault.
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn UsageFault() {
    loop {}
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn SecureFault() {
    loop {}
}

const ROM_VER: u32 = 1;

#[cfg(feature = "tz_support")]
unsafe fn branch_to_image(image: Image) -> ! {
    let sau_ctrl: *mut u32 = 0xe000edd0 as *mut u32;
    let sau_rbar: *mut u32 = 0xe000eddc as *mut u32;
    let sau_rlar: *mut u32 = 0xe000ede0 as *mut u32;
    let sau_rnr: *mut u32 = 0xe000edd8 as *mut u32;

    for i in 0..8 {
        if let Some(r) = image.get_sau_entry(i) {
            core::ptr::write_volatile(sau_rnr, i as u32);
            core::ptr::write_volatile(sau_rbar, r.rbar);
            core::ptr::write_volatile(sau_rlar, r.rlar);
        }
    }

    core::ptr::write_volatile(sau_ctrl, 1);

    let mut peripherals = Peripherals::steal();

    // let co processor be non-secure
    core::ptr::write_volatile(0xE000ED8C as *mut u32, 0xc00);

    peripherals
        .SCB
        .enable(cortex_m::peripheral::scb::Exception::UsageFault);
    peripherals
        .SCB
        .enable(cortex_m::peripheral::scb::Exception::BusFault);

    peripherals
        .SCB
        .enable(cortex_m::peripheral::scb::Exception::SecureFault);

    // Make our exceptions NS
    core::ptr::write_volatile(0xe000ed0c as *mut u32, 0x05fa2000);

    // Write the NS_VTOR
    core::ptr::write_volatile(0xE002ED08 as *mut u32, image.get_vectors());

    // Route all interrupts to the NS world
    // TODO use only the interrupts we've enabled
    core::ptr::write_volatile(0xe000e380 as *mut u32, 0xffffffff);
    core::ptr::write_volatile(0xe000e384 as *mut u32, 0xffffffff);

    // For secure we do not set the thumb bit!
    let entry_pt = image.get_pc() & !1u32;
    let stack = image.get_sp();

    // and branch
    arch::asm!("
            msr MSP_NS, {stack}
            bxns {entry}",
        stack = in(reg) stack,
        entry = in(reg) entry_pt,
        options(noreturn),
    );
}

#[cfg(not(feature = "tz_support"))]
unsafe fn branch_to_image(image: Image) -> ! {
    let mut peripherals = Peripherals::steal();

    peripherals
        .SCB
        .enable(cortex_m::peripheral::scb::Exception::UsageFault);
    peripherals
        .SCB
        .enable(cortex_m::peripheral::scb::Exception::BusFault);

    // Write the VTOR
    core::ptr::write_volatile(0xE000ED08 as *mut u32, image.get_vectors());

    let entry_pt = image.get_pc();
    let stack = image.get_sp();

    // and branch
    arch::asm!("
            msr MSP, {stack}
            bx {entry}",
        stack = in(reg) stack,
        entry = in(reg) entry_pt,
        options(noreturn),
    );
}

#[entry]
fn main() -> ! {
    // This is the SYSCON_DIEID register on LPC55 which contains the ROM
    // version. Make sure our configuration matches!
    let val = unsafe { core::ptr::read_volatile(0x50000ffc as *const u32) };

    if val & 1 != ROM_VER {
        panic!()
    }

    let (image, _) = image_header::select_image_to_boot();
    image_header::dump_image_details_to_ram();

    #[cfg(any(feature = "dice-mfg", feature = "dice-self"))]
    dice::run(&image);

    unsafe {
        branch_to_image(image);
    }
}

include!(concat!(env!("OUT_DIR"), "/consts.rs"));
