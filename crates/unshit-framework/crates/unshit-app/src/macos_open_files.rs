//! AppKit bridge for files opened through a bundled macOS application.
//!
//! Winit intentionally does not install an `NSApplicationDelegate`. Finder's
//! Open With actions therefore need this framework-owned delegate to turn
//! AppKit URLs into the ordinary app callback and rebuild wakeups.

use std::path::PathBuf;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationDelegateReply};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSURL};

use crate::{EventSink, ExternalEvent};

type OpenFilesCallback = Arc<dyn Fn(&[PathBuf]) -> bool + Send + Sync>;

#[derive(Clone)]
pub(crate) struct Ivars {
    callback: OpenFilesCallback,
    sink: EventSink,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    #[name = "UnshitOpenFilesDelegate"]
    pub(crate) struct OpenFilesDelegate;

    unsafe impl NSObjectProtocol for OpenFilesDelegate {}

    unsafe impl NSApplicationDelegate for OpenFilesDelegate {
        #[unsafe(method(application:openURLs:))]
        #[allow(non_snake_case)]
        fn application_openURLs(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            let mut paths = Vec::with_capacity(urls.count());
            for index in 0..urls.count() {
                let url = urls.objectAtIndex(index);
                if let Some(path) = url.filePathURL().and_then(|file_url| file_url.path()) {
                    paths.push(PathBuf::from(path.to_string()));
                }
            }
            self.open_paths(paths);
        }

        #[unsafe(method(application:openFile:))]
        #[allow(non_snake_case)]
        fn application_openFile(&self, _application: &NSApplication, filename: &NSString) -> bool {
            self.open_paths(vec![PathBuf::from(filename.to_string())])
        }

        #[unsafe(method(application:openFiles:))]
        #[allow(non_snake_case)]
        fn application_openFiles(
            &self,
            application: &NSApplication,
            filenames: &NSArray<NSString>,
        ) {
            let mut paths = Vec::with_capacity(filenames.count());
            for index in 0..filenames.count() {
                paths.push(PathBuf::from(filenames.objectAtIndex(index).to_string()));
            }
            let accepted = self.open_paths(paths);
            application.replyToOpenOrPrint(if accepted {
                NSApplicationDelegateReply::Success
            } else {
                NSApplicationDelegateReply::Failure
            });
        }
    }
);

impl OpenFilesDelegate {
    fn new(mtm: MainThreadMarker, callback: OpenFilesCallback, sink: EventSink) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { callback, sink });
        unsafe { msg_send![super(this), init] }
    }

    fn open_paths(&self, paths: Vec<PathBuf>) -> bool {
        if paths.is_empty() || !(self.ivars().callback)(&paths) {
            return false;
        }
        // Finder expects the existing app to come forward after it opens the
        // selected item. These use the framework queue so the callback's
        // mutation gets one coalesced rebuild rather than immediate repaint.
        let _ = self.ivars().sink.send(ExternalEvent::ActivateWindow);
        self.ivars().sink.send(ExternalEvent::RequestRebuild).is_ok()
    }
}

/// Install after winit created its event loop, then retain the return value
/// through `run_app`; AppKit holds its delegate weakly.
pub(crate) fn install(callback: OpenFilesCallback, sink: EventSink) -> Retained<OpenFilesDelegate> {
    let mtm =
        MainThreadMarker::new().expect("macOS application delegate must run on the main thread");
    let delegate = OpenFilesDelegate::new(mtm, callback, sink);
    let application = NSApplication::sharedApplication(mtm);
    application.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    delegate
}
