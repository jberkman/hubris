// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Client API for the User LEDs driver.

#![no_std]

use serde::{Deserialize, Serialize};
use userlib::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq, FromPrimitive)]
pub enum IdolTestError {
    UhOh = 1,
    YouAskedForThis = 2,
}
impl TryFrom<u32> for IdolTestError {
    type Error = ();
    fn try_from(x: u32) -> Result<Self, Self::Error> {
        Self::from_u32(x).ok_or(())
    }
}
impl From<IdolTestError> for u16 {
    fn from(rc: IdolTestError) -> Self {
        rc as u16
    }
}

#[derive(Copy, Clone, Serialize, Deserialize)]
pub struct FancyTestType {
    pub u: u32,
    pub b: bool,
    pub f: f32,
}

////////////////////////////////////////////////////////////////////////////////
// The structs below replicate an Idolatry bug related to serialization
// using ssmarshal

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum SocketName {
    Echo = 1,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ipv6Address(pub [u8; 16]);

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Address {
    Ipv6(Ipv6Address),
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct UdpMetadata {
    pub addr: Address,
    pub port: u16,
    pub size: u32,
    pub vid: u16,
}

include!(concat!(env!("OUT_DIR"), "/client_stub.rs"));
