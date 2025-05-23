//! Individually indexed receive buffers
//!
//! Messages can be placed in dedicated buffers by [`Filter::StoreBuffer`] or
//! [`ExtFilter::StoreBuffer`].
//!
//! [`Filter::StoreBuffer`]: crate::filter::Filter::StoreBuffer
//! [`ExtFilter::StoreBuffer`]: crate::filter::ExtFilter::StoreBuffer

use crate::message::rx;
use crate::reg;
use core::convert::Infallible;
use core::marker::PhantomData;
use reg::AccessRegisterBlock as _;
use vcell::VolatileCell;

/// Index is out of bounds
#[derive(Debug)]
pub struct OutOfBounds;

/// Dedicated receive buffers on peripheral `P`
pub struct RxDedicatedBuffer<'a, P, M: rx::AnyMessage> {
    memory: &'a mut [VolatileCell<M>],
    _markers: PhantomData<P>,
}

/// Trait which erases generic parametrization for [`RxDedicatedBuffer`] type
pub trait DynRxDedicatedBuffer {
    /// CAN identity type
    type Id;

    /// Received message type
    type Message;

    /// Returns a received frame from the selected buffer if available
    fn receive(&mut self, index: usize) -> nb::Result<Self::Message, OutOfBounds>;

    /// Returns a received frame from any dedicated buffer if available
    fn receive_any(&mut self) -> nb::Result<Self::Message, Infallible>;
}

impl<'a, P: mcan_core::CanId, M: rx::AnyMessage> RxDedicatedBuffer<'a, P, M> {
    /// # Safety
    /// The caller must be the owner or the peripheral referenced by `P`. The
    /// constructed type assumes ownership of some of the registers from the
    /// peripheral `RegisterBlock`. Do not use them to avoid aliasing. Do not
    /// keep multiple instances for the same peripheral.
    /// - NDAT1
    /// - NDAT2
    pub(crate) unsafe fn new(memory: &'a mut [VolatileCell<M>]) -> Self {
        Self {
            memory,
            _markers: PhantomData,
        }
    }

    /// Raw access to the registers.
    unsafe fn regs(&self) -> &reg::RegisterBlock {
        &(*P::register_block())
    }

    fn ndat1(&self) -> &reg::NDAT1 {
        // Safety: `Self` owns the register.
        unsafe { &self.regs().ndat1 }
    }

    fn ndat2(&self) -> &reg::NDAT2 {
        // Safety: `Self` owns the register.
        unsafe { &self.regs().ndat2 }
    }

    fn has_new_data(&self, index: usize) -> bool {
        if index < 32 {
            self.ndat1().read().bits() & (1 << index) != 0
        } else if index < 64 {
            self.ndat2().read().bits() & (1 << (index - 32)) != 0
        } else {
            false
        }
    }

    fn has_new_data_checked(&self, index: usize) -> Result<bool, OutOfBounds> {
        if index < 64 {
            Ok(self.has_new_data(index))
        } else {
            Err(OutOfBounds)
        }
    }

    fn mark_buffer_read(&self, index: usize) {
        if index < 32 {
            unsafe {
                self.ndat1().write(|w| w.bits(1 << index));
            }
        } else if index < 64 {
            unsafe {
                self.ndat2().write(|w| w.bits(1 << index));
            }
        }
    }

    fn peek(&self, index: usize) -> nb::Result<M, OutOfBounds> {
        if self.has_new_data_checked(index)? {
            Ok(self
                .memory
                .get(index)
                .ok_or(nb::Error::Other(OutOfBounds))?
                .get())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}

impl<P: mcan_core::CanId, M: rx::AnyMessage> DynRxDedicatedBuffer for RxDedicatedBuffer<'_, P, M> {
    type Id = P;
    type Message = M;

    fn receive(&mut self, index: usize) -> nb::Result<Self::Message, OutOfBounds> {
        let message = self.peek(index)?;
        self.mark_buffer_read(index);
        Ok(message)
    }

    fn receive_any(&mut self) -> nb::Result<Self::Message, Infallible> {
        self.memory
            .iter()
            .enumerate()
            .filter(|&(i, _)| self.has_new_data(i))
            .map(|(i, m)| (i, m.get()))
            .min_by_key(|(_, m)| m.id())
            .map(|(i, m)| {
                self.mark_buffer_read(i);
                m
            })
            .ok_or(nb::Error::WouldBlock)
    }
}

impl<P: mcan_core::CanId, M: rx::AnyMessage> Iterator for RxDedicatedBuffer<'_, P, M> {
    type Item = M;

    fn next(&mut self) -> Option<Self::Item> {
        self.receive_any().ok()
    }
}
