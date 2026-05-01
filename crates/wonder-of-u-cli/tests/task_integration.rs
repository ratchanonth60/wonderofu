use std::{fs, path::Path, time::Duration};

use tempfile::TempDir;
use tokio::time::sleep;
use wonder_of_u_cli::run_from;
use wonder_of_u_core::{TaskId, TaskStatus};
use wonder_of_u_storage::TaskStore;

fn extract_value<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing prefix {prefix} in {text}"))
}

fn run_cli(args: Vec<String>) -> String {
    let mut output = Vec::new();
    run_from(args, &mut output).expect("run cli");
    String::from_utf8(output).expect("utf8 output")
}

fn task_start_output(storage_dir: &Path, cwd: &Path, description: &str, command: &str) -> String {
    run_cli(vec![
        "wonder-of-u".into(),
        "--storage-dir".into(),
        storage_dir.display().to_string(),
        "tasks".into(),
        "start".into(),
        "shell".into(),
        "--description".into(),
        description.into(),
        "--command".into(),
        command.into(),
        "--cwd".into(),
        cwd.display().to_string(),
        "--read-only".into(),
    ])
}

fn reconcile_task(storage_dir: &Path, task_id: TaskId) {
    let _ = run_cli(vec![
        "wonder-of-u".into(),
        "--storage-dir".into(),
        storage_dir.display().to_string(),
        "tasks".into(),
        "show".into(),
        task_id.to_string(),
        "--tail".into(),
        "50".into(),
    ]);
}

async fn wait_for(mut condition: impl FnMut() -> bool, failure_message: &str) {
    for _ in 0..50 {
        if condition() {
            return;
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("{failure_message}");
}

#[cfg(unix)]
fn process_has_stopped(pid: u32) -> bool {
    let signal_result = unsafe { libc::kill(pid as i32, 0) };
    if signal_result == -1 {
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    }
    let Ok(stat) = fs::read_to_string(Path::new(&format!("/proc/{pid}/stat"))) else {
        return true;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        == Some("Z")
}

#[tokio::test]
async fn integration_task_spawn_and_stream_output() {
    let storage_dir = TempDir::new().expect("temp storage");
    let cwd = std::env::current_dir().expect("current dir");
    let output = task_start_output(
        storage_dir.path(),
        &cwd,
        "stream output",
        "printf 'hello\\n'; sleep 0.2; printf 'world\\n'",
    );
    let task_id = TaskId::parse(extract_value(&output, "task_id=")).expect("task id");
    let store = TaskStore::new(storage_dir.path());

    wait_for(
        || {
            let log = store.read_log(task_id).unwrap_or_default();
            log.contains("hello\n") && !log.contains("world\n")
        },
        "expected first output line before task completed",
    )
    .await;

    wait_for(
        || {
            reconcile_task(storage_dir.path(), task_id);
            store
                .read_task(task_id)
                .map(|task| task.status == TaskStatus::Completed)
                .unwrap_or(false)
        },
        "expected task to complete",
    )
    .await;

    let log = store.read_log(task_id).expect("read final log");
    let hello = log.find("hello\n").expect("hello line");
    let world = log.find("world\n").expect("world line");
    assert!(hello < world, "expected hello before world in {log}");
}

#[tokio::test]
async fn integration_task_cancel_mid_run() {
    let storage_dir = TempDir::new().expect("temp storage");
    let cwd = std::env::current_dir().expect("current dir");
    let output = task_start_output(storage_dir.path(), &cwd, "cancel task", "sleep 60");
    let task_id = TaskId::parse(extract_value(&output, "task_id=")).expect("task id");
    let store = TaskStore::new(storage_dir.path());

    sleep(Duration::from_millis(100)).await;

    let stop_output = run_cli(vec![
        "wonder-of-u".into(),
        "--storage-dir".into(),
        storage_dir.path().display().to_string(),
        "tasks".into(),
        "stop".into(),
        task_id.to_string(),
    ]);
    assert_eq!(extract_value(&stop_output, "status="), "cancelled");

    let task = store.read_task(task_id).expect("read stopped task");
    assert_eq!(task.status, TaskStatus::Cancelled);

    #[cfg(unix)]
    {
        let pid = task.pid.expect("persisted pid");
        wait_for(
            || process_has_stopped(pid),
            "expected cancelled task process to exit",
        )
        .await;
    }
}

#[tokio::test]
async fn integration_task_output_is_persisted() {
    let storage_dir = TempDir::new().expect("temp storage");
    let cwd = std::env::current_dir().expect("current dir");
    let output = task_start_output(
        storage_dir.path(),
        &cwd,
        "persisted output",
        "printf 'persisted line\\n'",
    );
    let task_id = TaskId::parse(extract_value(&output, "task_id=")).expect("task id");
    let store = TaskStore::new(storage_dir.path());

    wait_for(
        || {
            reconcile_task(storage_dir.path(), task_id);
            store
                .read_task(task_id)
                .map(|task| task.status == TaskStatus::Completed)
                .unwrap_or(false)
        },
        "expected task to complete",
    )
    .await;

    let task = store.read_task(task_id).expect("completed task");
    let log_path = task.output_log.expect("output log path");
    let persisted = fs::read_to_string(&log_path).expect("read persisted output log");
    assert!(persisted.contains("persisted line"));

    let streamed = store.read_log(task_id).expect("read streamed log");
    assert_eq!(persisted, streamed);
}
