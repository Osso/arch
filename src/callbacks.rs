use crate::journal;
use alpm::{Alpm, Event, LogLevel, PackageOperation};

fn handle_log_message(level: LogLevel, msg: &str) {
    match level {
        LogLevel::ERROR => {
            eprint!("error: {}", msg);
            journal::log_error(msg.trim());
        }
        LogLevel::WARNING => {
            eprint!("warning: {}", msg);
            journal::log_warning(msg.trim());
        }
        LogLevel::DEBUG => {}
        LogLevel::FUNCTION => {}
        _ => print!("{}", msg),
    }
}

fn package_operation_details(
    operation: PackageOperation<'_>,
) -> (&'static str, &str, &str, Option<&str>) {
    match operation {
        PackageOperation::Install(pkg) => ("install", pkg.name(), pkg.version().as_str(), None),
        PackageOperation::Upgrade(old, new) => (
            "upgrade",
            new.name(),
            new.version().as_str(),
            Some(old.version().as_str()),
        ),
        PackageOperation::Reinstall(_, new) => {
            ("reinstall", new.name(), new.version().as_str(), None)
        }
        PackageOperation::Downgrade(old, new) => (
            "downgrade",
            new.name(),
            new.version().as_str(),
            Some(old.version().as_str()),
        ),
        PackageOperation::Remove(pkg) => ("remove", pkg.name(), pkg.version().as_str(), None),
    }
}

fn log_package_operation(
    operation: &str,
    pkg_name: &str,
    version: &str,
    old_version: Option<&str>,
) {
    println!("{} {}...", operation, pkg_name);

    if let Some(old_ver) = old_version {
        journal::log_upgrade(pkg_name, old_ver, version);
    } else {
        journal::log_operation(operation, pkg_name, version);
    }
}

fn handle_event_message(event: Event<'_>) {
    match event {
        Event::PackageOperationStart(op) => {
            let (operation, pkg_name, version, old_version) =
                package_operation_details(op.operation());
            log_package_operation(operation, pkg_name, version, old_version);
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
}

/// Register callbacks for logging and events
pub fn register(handle: &Alpm) {
    // Log callback - for warnings/errors, also log to journal
    handle.set_log_cb((), |level, msg, _| {
        handle_log_message(level, msg);
    });

    // Event callback - log package operations to journald with structured fields
    handle.set_event_cb((), |event, _| {
        handle_event_message(event.event());
    });
}
