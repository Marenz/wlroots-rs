//! TODO Documentation
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};

use wlroots_sys::{wlr_input_device, wlr_keyboard, wlr_keyboard_get_modifiers, wlr_keyboard_led,
                  wlr_keyboard_led_update, wlr_keyboard_modifier, wlr_keyboard_set_keymap,
                  xkb_keymap};

use InputDevice;

#[derive(Debug)]
pub struct Keyboard {
    /// The structure that ensures weak handles to this structure are still alive.
    ///
    /// They contain weak handles, and will safely not use dead memory when this
    /// is freed by wlroots.
    ///
    /// If this is `None`, then this is from an upgraded `KeyboardHandle`, and
    /// the operations are **unchecked**.
    /// This is means safe operations might fail, but only if you use the unsafe
    /// marked function `upgrade` on a `KeyboardHandle`.
    liveliness: Option<Rc<AtomicBool>>,
    /// The device that refers to this keyboard.
    device: InputDevice,
    /// The underlying keyboard data.
    keyboard: *mut wlr_keyboard
}

#[derive(Debug)]
pub struct KeyboardHandle {
    /// The Rc that ensures that this handle is still alive.
    ///
    /// When wlroots deallocates the keyboard associated with this handle,
    handle: Weak<AtomicBool>,
    /// The device that refers to this keyboard.
    device: InputDevice,
    /// The underlying keyboard data.
    keyboard: *mut wlr_keyboard
}

impl Keyboard {
    /// Tries to convert an input device to a Keyboard
    ///
    /// Returns None if it is of a different type of input variant.
    ///
    /// # Safety
    /// This creates a totally new Keyboard (e.g with its own reference count)
    /// so only do this once per `wlr_input_device`!
    pub(crate) unsafe fn new_from_input_device(device: *mut wlr_input_device) -> Option<Self> {
        use wlroots_sys::wlr_input_device_type::*;
        match (*device).type_ {
            WLR_INPUT_DEVICE_KEYBOARD => {
                let keyboard = (*device).__bindgen_anon_1.keyboard;
                Some(Keyboard { liveliness: Some(Rc::new(AtomicBool::new(false))),
                                device: InputDevice::from_ptr(device),
                                keyboard })
            }
            _ => None
        }
    }

    unsafe fn from_handle(handle: &KeyboardHandle) -> Self {
        Keyboard { liveliness: None,
                   device: handle.input_device().clone(),
                   keyboard: handle.as_ptr() }
    }

    /// Gets the wlr_keyboard associated with this KeyboardHandle.
    pub unsafe fn as_ptr(&self) -> *mut wlr_keyboard {
        self.keyboard
    }

    /// Gets the wlr_input_device associated with this KeyboardHandle
    pub fn input_device(&self) -> &InputDevice {
        &self.device
    }

    // TODO: Implement keymap wrapper?
    pub fn set_keymap(&mut self, keymap: *mut xkb_keymap) {
        unsafe {
            wlr_keyboard_set_keymap(self.keyboard, keymap);
        }
    }

    pub fn update_led(&mut self, leds: KeyboardLed) {
        unsafe {
            wlr_keyboard_led_update(self.keyboard, leds.bits() as u32);
        }
    }

    pub fn get_modifiers(&self) -> KeyboardModifier {
        unsafe {
            KeyboardModifier::from_bits_truncate(wlr_keyboard_get_modifiers(self.keyboard))
        }
    }

    /// Creates a weak reference to a `Keyboard`.
    ///
    /// # Panics
    /// If this `Keyboard` is a previously upgraded `KeyboardHandle`,
    /// then this function will panic.
    pub fn weak_reference(&self) -> KeyboardHandle {
        let arc = self.liveliness.as_ref()
                      .expect("Cannot downgrade previously upgraded KeyboardHandle!");
        KeyboardHandle { handle: Rc::downgrade(arc),
                         // NOTE Rationale for cloning:
                         // We can't use the keyboard handle unless the keyboard is alive,
                         // which means the device pointer is still alive.
                         device: unsafe { self.device.clone() },
                         keyboard: self.keyboard }
    }

    /// Manually set the lock used to determine if a double-borrow is
    /// occuring on this structure.
    ///
    /// # Panics
    /// Panics when trying to set the lock on an upgraded handle.
    pub(crate) unsafe fn set_lock(&self, val: bool) {
        self.liveliness.as_ref()
            .expect("Tried to set lock on borrowed Keyboard")
            .store(val, Ordering::Release);
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        match self.liveliness {
            None => {}
            Some(ref liveliness) => {
                if Rc::strong_count(liveliness) == 1 {
                    wlr_log!(L_DEBUG, "Dropped Keyboard {:p}", self.keyboard);
                    let weak_count = Rc::weak_count(liveliness);
                    if weak_count > 0 {
                        wlr_log!(L_DEBUG,
                                 "Still {} weak pointers to Keyboard {:p}",
                                 weak_count,
                                 self.keyboard);
                    }
                }
            }
        }
    }
}

impl KeyboardHandle {
    /// Upgrades the keyboard handle to a reference to the backing `Keyboard`.
    ///
    /// # Unsafety
    /// This function is unsafe, because it creates an unbounded `Keyboard`
    /// which may live forever..
    /// But no keyboard lives forever and might be disconnected at any time.
    ///
    /// # Panics
    /// This function will panic if multiple mutable borrows are detected.
    pub(crate) unsafe fn upgrade(&self) -> Option<Keyboard> {
        self.handle.upgrade()
        // NOTE
        // We drop the Rc here because having two would allow a dangling
        // pointer to exist!
            .map(|check| {
                let keyboard = Keyboard::from_handle(self);
                if check.load(Ordering::Acquire) {
                    wlr_log!(L_ERROR, "Double mutable borrows on {:?}", keyboard);
                    panic!("Double mutable borrows detected");
                }
                check.store(true, Ordering::Release);
                keyboard
            })
    }

    /// Run a function on the referenced Keyboard, if it still exists
    ///
    /// Returns the result of the function, if successful
    ///
    /// # Safety
    /// By enforcing a rather harsh limit on the lifetime of the output
    /// to a short lived scope of an anonymous function,
    /// this function ensures the Keyboard does not live longer
    /// than it exists.
    ///
    /// # Panics
    /// This function will panic if multiple mutable borrows are detected.
    /// This will happen if you call `upgrade` directly within this callback,
    /// or if you run this function within the another run to the same `Output`.
    ///
    /// So don't nest `run` calls and everything will be ok :).
    pub fn run<F, R>(&mut self, runner: F) -> Option<R>
        where F: FnOnce(&mut Keyboard) -> R
    {
        let mut keyboard = unsafe { self.upgrade() };
        match keyboard {
            None => None,
            Some(ref mut keyboard) => {
                let res = Some(runner(keyboard));
                self.handle.upgrade().map(|check| {
                    // Sanity check that it hasn't been tampered with.
                    if !check.load(Ordering::Acquire) {
                        wlr_log!(L_ERROR, "After running keyboard callback, \
                                           mutable lock was false for: {:?}", keyboard);
                        panic!("Lock in incorrect state!");
                    }
                    check.store(false, Ordering::Release);
                });
                res
            }
        }
    }

    /// Gets the wlr_input_device associated with this KeyboardHandle
    pub fn input_device(&self) -> &InputDevice {
        &self.device
    }

    /// Gets the wlr_keyboard associated with this KeyboardHandle.
    pub unsafe fn as_ptr(&self) -> *mut wlr_keyboard {
        self.keyboard
    }
}

bitflags! {
    pub struct KeyboardLed: u32 {
        const WLR_LED_NUM_LOCK = wlr_keyboard_led::WLR_LED_NUM_LOCK as u32;
        const WLR_LED_CAPS_LOCK = wlr_keyboard_led::WLR_LED_CAPS_LOCK as u32;
        const WLR_LED_SCROLL_LOCK = wlr_keyboard_led::WLR_LED_SCROLL_LOCK as u32;
    }
}

bitflags! {
    pub struct KeyboardModifier: u32 {
        const WLR_MODIFIER_SHIFT = wlr_keyboard_modifier::WLR_MODIFIER_SHIFT as u32;
        const WLR_MODIFIER_CAPS = wlr_keyboard_modifier::WLR_MODIFIER_CAPS as u32;
        const WLR_MODIFIER_CTRL = wlr_keyboard_modifier::WLR_MODIFIER_CTRL as u32;
        const WLR_MODIFIER_ALT = wlr_keyboard_modifier::WLR_MODIFIER_ALT as u32;
        const WLR_MODIFIER_MOD2 = wlr_keyboard_modifier::WLR_MODIFIER_MOD2 as u32;
        const WLR_MODIFIER_MOD3 = wlr_keyboard_modifier::WLR_MODIFIER_MOD3 as u32;
        const WLR_MODIFIER_LOGO = wlr_keyboard_modifier::WLR_MODIFIER_LOGO as u32;
        const WLR_MODIFIER_MOD5 = wlr_keyboard_modifier::WLR_MODIFIER_MOD5 as u32;
    }
}

impl fmt::Display for KeyboardModifier {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let mod_vec = vec![("Shift", KeyboardModifier::WLR_MODIFIER_SHIFT),
                           ("Caps lock", KeyboardModifier::WLR_MODIFIER_CAPS),
                           ("Ctrl", KeyboardModifier::WLR_MODIFIER_CTRL),
                           ("Alt", KeyboardModifier::WLR_MODIFIER_ALT),
                           ("Mod2", KeyboardModifier::WLR_MODIFIER_MOD2),
                           ("Mod3", KeyboardModifier::WLR_MODIFIER_MOD3),
                           ("Logo", KeyboardModifier::WLR_MODIFIER_LOGO),
                           ("Mod5", KeyboardModifier::WLR_MODIFIER_MOD5)];

        let mods: Vec<&str> = mod_vec.into_iter()
                                     .filter(|&(_, flag)| self.contains(flag))
                                     .map(|(st, _)| st)
                                     .collect();

        write!(formatter, "{:?}", mods)
    }
}
