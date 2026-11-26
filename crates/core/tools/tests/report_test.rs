use std::process::ExitCode;

use merc_tools::report_error;

#[test]
fn test_report_error_exit_code() {
    assert_eq!(report_error(Ok(())), ExitCode::SUCCESS);
    assert_eq!(report_error(Err("failure".into())), ExitCode::FAILURE);
}
