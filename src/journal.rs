use libsystemd::logging::{journal_send, Priority};

/// Log a package operation to journald with structured fields
pub fn log_operation(operation: &str, package: &str, version: &str) {
    let fields = [
        ("SYSLOG_IDENTIFIER", "arch"),
        ("OPERATION", operation),
        ("PACKAGE", package),
        ("VERSION", version),
    ];

    let msg = format!("{} {} {}", operation, package, version);
    let _ = journal_send(Priority::Info, &msg, fields.into_iter());
}

/// Log a package upgrade with old and new versions
pub fn log_upgrade(package: &str, old_version: &str, new_version: &str) {
    let fields = [
        ("SYSLOG_IDENTIFIER", "arch"),
        ("OPERATION", "upgrade"),
        ("PACKAGE", package),
        ("OLD_VERSION", old_version),
        ("VERSION", new_version),
    ];

    let msg = format!("upgrade {} {} -> {}", package, old_version, new_version);
    let _ = journal_send(Priority::Info, &msg, fields.into_iter());
}

/// Log transaction start
pub fn log_transaction_start() {
    let fields = [
        ("SYSLOG_IDENTIFIER", "arch"),
        ("OPERATION", "transaction_start"),
    ];

    let _ = journal_send(Priority::Info, "transaction started", fields.into_iter());
}

/// Log transaction complete
pub fn log_transaction_complete() {
    let fields = [
        ("SYSLOG_IDENTIFIER", "arch"),
        ("OPERATION", "transaction_complete"),
    ];

    let _ = journal_send(Priority::Info, "transaction completed", fields.into_iter());
}

/// Log a warning
pub fn log_warning(msg: &str) {
    let fields = [("SYSLOG_IDENTIFIER", "arch")];
    let _ = journal_send(Priority::Warning, msg, fields.into_iter());
}

/// Log an error
pub fn log_error(msg: &str) {
    let fields = [("SYSLOG_IDENTIFIER", "arch")];
    let _ = journal_send(Priority::Error, msg, fields.into_iter());
}
