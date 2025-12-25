use crate::journal;
use alpm::{Alpm, Event, LogLevel, PackageOperation};

/// Register callbacks for logging and events
pub fn register(handle: &Alpm) {
    // Log callback - for warnings/errors, also log to journal
    handle.set_log_cb((), |level, msg, _| {
        match level {
            LogLevel::ERROR => {
                eprint!("error: {}", msg);
                journal::log_error(msg.trim());
            }
            LogLevel::WARNING => {
                eprint!("warning: {}", msg);
                journal::log_warning(msg.trim());
            }
            LogLevel::DEBUG => {} // Skip debug messages
            LogLevel::FUNCTION => {} // Skip function traces
            _ => print!("{}", msg),
        }
    });

    // Event callback - log package operations to journald with structured fields
    handle.set_event_cb((), |event, _| {
        match event.event() {
            Event::PackageOperationStart(op) => {
                let (operation, pkg_name, version, old_version) = match op.operation() {
                    PackageOperation::Install(pkg) => {
                        ("install", pkg.name(), pkg.version().as_str(), None)
                    }
                    PackageOperation::Upgrade(old, new) => {
                        ("upgrade", new.name(), new.version().as_str(), Some(old.version().as_str()))
                    }
                    PackageOperation::Reinstall(_, new) => {
                        ("reinstall", new.name(), new.version().as_str(), None)
                    }
                    PackageOperation::Downgrade(old, new) => {
                        ("downgrade", new.name(), new.version().as_str(), Some(old.version().as_str()))
                    }
                    PackageOperation::Remove(pkg) => {
                        ("remove", pkg.name(), pkg.version().as_str(), None)
                    }
                };

                // Print to terminal
                println!("{} {}...", operation, pkg_name);

                // Log to journald with structured fields
                if let Some(old_ver) = old_version {
                    journal::log_upgrade(pkg_name, old_ver, version);
                } else {
                    journal::log_operation(operation, pkg_name, version);
                }
            }
            Event::PackageOperationDone(_) => {}
            Event::TransactionStart => {
                println!(":: Starting transaction...");
                journal::log_transaction_start();
            }
            Event::TransactionDone => {
                println!(":: Transaction completed.");
                journal::log_transaction_complete();
            }
            Event::RetrieveStart => println!(":: Downloading..."),
            Event::RetrieveDone => {}
            Event::PkgRetrieveStart(dl) => {
                print!("Downloading {}...", dl.num());
            }
            Event::PkgRetrieveDone(_) => println!(" done"),
            Event::PkgRetrieveFailed(_) => println!(" failed"),
            _ => {}
        }
    });
}
