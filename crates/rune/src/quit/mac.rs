//! AppKit asks the app delegate `applicationShouldTerminate:` before every quit: ⌘Q, the Dock's
//! Quit, logout, restart and shutdown. winit's delegate doesn't implement it, so Rune adds the
//! method to the delegate's class once (as `platform::reopen` does for reopening). Consent
//! returns Later until Rune's loop answers; without a hook it permits Quit as before.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};

use block2::{Block, RcBlock};
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::NSApplication;

#[derive(Clone, Copy)]
enum Install {
    Unknown,
    Hooked(*const AnyClass),
    OtherDelegate,
    Warned,
}

thread_local! {
    static INSTALLED: Cell<Install> = const { Cell::new(Install::Unknown) };
    static SEND: RefCell<Option<Box<dyn Fn() -> bool>>> = const { RefCell::new(None) };
}

unsafe extern "C" {
    fn class_addMethod(class: *const AnyClass, selector: Sel, implementation: *const c_void, types: *const c_char) -> Bool;
}

extern "C" fn should_terminate(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> usize {
    std::panic::catch_unwind(|| {
        let reply = super::request::request(|| {
            SEND.with(|s| s.borrow().as_ref().is_some_and(|send| send()))
        });
        if reply == super::request::NativeReply::Now {
            // Windows::exiting carries out the quit's effects.
            super::send(super::Event::Start {
                ending: false,
                after: super::After::Exit,
            });
        }
        reply as usize
    })
    .unwrap_or_else(|_| {
        eprintln!("rune: application Quit callback panicked");
        0
    })
}

pub(super) fn install(send: impl Fn() -> bool + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let selector = sel!(applicationShouldTerminate:);
    // SAFETY: AppKit calls and runtime changes on the main thread. The added function has the
    // selector's ABI (`NSApplicationTerminateReply` is an `NSUInteger`, `Q` on 64-bit), and
    // adding a method the class lacks leaves its other methods and instances alone.
    unsafe {
        let delegate: *mut AnyObject = msg_send![&app, delegate];
        let Some(delegate) = delegate.as_ref() else { return };
        let class = delegate.class();
        if INSTALLED.with(|s| matches!(s.get(), Install::Hooked(c) if std::ptr::eq(c, class))) {
            SEND.with(|s| *s.borrow_mut() = Some(Box::new(send)));
            return;
        }
        // A later winit, or an app's own delegate, decides itself.
        let answers: bool = msg_send![delegate, respondsToSelector: selector];
        if answers {
            INSTALLED.with(|s| {
                if !matches!(s.get(), Install::Warned) {
                    s.set(Install::OtherDelegate);
                }
            });
            warn_if_unavailable();
            return;
        }
        let imp = should_terminate as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize;
        if !class_addMethod(class, selector, imp as *const c_void, c"Q@:@".as_ptr()).as_bool() {
            eprintln!("rune: could not add the terminate method to the app delegate");
            return;
        }
        INSTALLED.with(|s| s.set(Install::Hooked(class)));
        // AppKit caches which optional methods its delegate answers when it is set.
        let _: () = msg_send![&app, setDelegate: delegate];
    }
    SEND.with(|s| *s.borrow_mut() = Some(Box::new(send)));
}

pub(super) fn warn_if_unavailable() {
    INSTALLED.with(|s| if matches!(s.get(), Install::OtherDelegate) && super::request::registered() {
        eprintln!("rune: application Quit consent unavailable: the app delegate already handles termination");
        s.set(Install::Warned);
    });
}

pub(super) fn reply(allow: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if allow {
        super::send(super::Event::Start {
            ending: false,
            after: super::After::Exit,
        });
    }
    // The reply can call applicationWillTerminate synchronously.
    NSApplication::sharedApplication(mtm).replyToApplicationShouldTerminate(allow);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopPerformBlock(run_loop: *mut c_void, mode: *const c_void, block: &Block<dyn Fn()>);
    fn CFRunLoopWakeUp(run_loop: *mut c_void);
}

pub(super) fn schedule_reply() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let block = RcBlock::new(|| {
        if std::panic::catch_unwind(super::request::flush).is_err() {
            eprintln!("rune: application Quit reply panicked");
        }
    });
    // SAFETY: the process's main run loop and common modes are valid; PerformBlock copies
    // the block. Completion also checks the handler stack before replying to AppKit.
    unsafe {
        let run_loop = CFRunLoopGetMain();
        CFRunLoopPerformBlock(run_loop, kCFRunLoopCommonModes, &block);
        CFRunLoopWakeUp(run_loop);
    }
}
