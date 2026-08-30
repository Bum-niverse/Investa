use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{
    ml_pipeline::{
        complete_training_job, shard_payload, training_job_source, MlArtifactDescriptor,
        MlShardWorkerBundle, MlShardWorkerFile, MlTrainingJobCompleteRequest,
        MlTrainingJobCompletion, MlTrainingJobSource,
    },
    persistence::{self, PersistenceBridge},
};

const STDOUT_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const STDERR_LIMIT_BYTES: usize = 256 * 1024;
const RESULT_LIMIT_BYTES: u64 = 16 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(25);
static RUNNING_JOBS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();

struct RunningJobGuard {
    job_id: String,
}

impl RunningJobGuard {
    fn claim(job_id: &str) -> Result<Self, String> {
        let running = RUNNING_JOBS.get_or_init(|| Mutex::new(BTreeSet::new()));
        let mut jobs = running
            .lock()
            .map_err(|_| "ML worker 실행 잠금을 사용할 수 없습니다.".to_owned())?;
        if !jobs.insert(job_id.to_owned()) {
            return Err("동일 ML 작업이 이미 실행 중입니다.".to_owned());
        }
        Ok(Self {
            job_id: job_id.to_owned(),
        })
    }
}

impl Drop for RunningJobGuard {
    fn drop(&mut self) {
        if let Some(running) = RUNNING_JOBS.get() {
            if let Ok(mut jobs) = running.lock() {
                jobs.remove(&self.job_id);
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlWorkerRunResult {
    pub completion: MlTrainingJobCompletion,
    pub elapsed_ms: u64,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub memory_limit_enforced: bool,
    pub live_order_allowed: bool,
}

#[derive(Debug)]
struct WorkerPaths {
    python: PathBuf,
    script: PathBuf,
    run_root: PathBuf,
}

#[derive(Debug)]
struct CappedOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

#[derive(Debug)]
struct ProcessOutcome {
    success: bool,
    timed_out: bool,
    stdout: CappedOutput,
    stderr: CappedOutput,
    elapsed_ms: u64,
    memory_limit_enforced: bool,
}

fn read_capped<R: Read>(mut reader: R, limit: usize) -> CappedOutput {
    let mut kept = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut exceeded = false;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..count.min(remaining)]);
                exceeded |= count > remaining;
            }
        }
    }
    CappedOutput {
        bytes: kept,
        exceeded,
    }
}

fn allowed_environment_values(run_dir: &Path, max_threads: u8) -> Vec<(OsString, OsString)> {
    let mut values = Vec::new();
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            values.push((OsString::from(key), value));
        }
    }
    let thread_limit = max_threads.to_string();
    values.extend([
        (OsString::from("TEMP"), run_dir.as_os_str().to_os_string()),
        (OsString::from("TMP"), run_dir.as_os_str().to_os_string()),
        (OsString::from("PYTHONUTF8"), OsString::from("1")),
        (OsString::from("PYTHONNOUSERSITE"), OsString::from("1")),
        (
            OsString::from("OMP_NUM_THREADS"),
            OsString::from(&thread_limit),
        ),
        (
            OsString::from("OPENBLAS_NUM_THREADS"),
            OsString::from(&thread_limit),
        ),
        (
            OsString::from("MKL_NUM_THREADS"),
            OsString::from(&thread_limit),
        ),
        (
            OsString::from("NUMEXPR_NUM_THREADS"),
            OsString::from(thread_limit),
        ),
    ]);
    values
}

fn allowed_environment(command: &mut Command, run_dir: &Path, max_threads: u8) {
    command.env_clear();
    command.envs(allowed_environment_values(run_dir, max_threads));
}

fn wait_for_child(child: &mut Child, timeout: Duration) -> Result<(bool, bool), String> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| "ML worker 종료 상태를 확인하지 못했습니다.".to_owned())?
        {
            return Ok((status.success(), false));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((false, true));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn run_process(
    executable: &Path,
    args: &[OsString],
    run_dir: &Path,
    timeout: Duration,
    memory_limit_mb: u32,
    max_threads: u8,
) -> Result<ProcessOutcome, String> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(run_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    allowed_environment(&mut command, run_dir, max_threads);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|_| "고정 ML worker 프로세스를 시작하지 못했습니다.".to_owned())?;
    let job = match WindowsJob::assign(&child, memory_limit_mb) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ML worker stdout을 캡처하지 못했습니다.".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ML worker stderr를 캡처하지 못했습니다.".to_owned())?;
    let stdout_reader = thread::spawn(move || read_capped(stdout, STDOUT_LIMIT_BYTES));
    let stderr_reader = thread::spawn(move || read_capped(stderr, STDERR_LIMIT_BYTES));
    let (success, timed_out) = wait_for_child(&mut child, timeout)?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| "ML worker stdout 수집기가 종료되었습니다.".to_owned())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "ML worker stderr 수집기가 종료되었습니다.".to_owned())?;
    let memory_limit_enforced = job.is_enforced();
    drop(job);
    Ok(ProcessOutcome {
        success,
        timed_out,
        stdout,
        stderr,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        memory_limit_enforced,
    })
}

fn resolve_paths(app: &AppHandle) -> Result<WorkerPaths, String> {
    let local_data = app
        .path()
        .local_data_dir()
        .map_err(|_| "로컬 ML 실행 경로를 확인하지 못했습니다.".to_owned())?;
    let python = local_data
        .join("Investa")
        .join("ml-worker-venv")
        .join("Scripts")
        .join("python.exe");
    let packaged = app
        .path()
        .resource_dir()
        .map_err(|_| "앱 ML resource 경로를 확인하지 못했습니다.".to_owned())?
        .join("ml-worker")
        .join("investa_ml_worker.py");
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "개발 ML worker 경로를 확인하지 못했습니다.".to_owned())?
        .join("ml-worker")
        .join("investa_ml_worker.py");
    let script = if packaged.is_file() {
        packaged
    } else if cfg!(debug_assertions) && development.is_file() {
        development
    } else {
        return Err("서명된 앱 resource의 ML worker를 찾지 못했습니다.".to_owned());
    };
    if !python.is_file() {
        return Err("Investa 전용 Python ML 환경이 준비되지 않았습니다.".to_owned());
    }
    Ok(WorkerPaths {
        python,
        script,
        run_root: local_data.join("Investa").join("ml-runs"),
    })
}

fn fail_job(
    bridge: &PersistenceBridge,
    job_id: &str,
    input_sha256: &str,
    failure_code: &str,
    completed_at_ms: u64,
) -> Result<MlTrainingJobCompletion, String> {
    complete_training_job(
        bridge,
        MlTrainingJobCompleteRequest {
            job_id: job_id.to_owned(),
            input_sha256: input_sha256.to_owned(),
            completed_at_ms,
            succeeded: false,
            failure_code: Some(failure_code.to_owned()),
            model_id: None,
            model_version: None,
            artifact: None,
            metrics: None,
            predictions: None,
        },
    )
}

fn verify_artifact(run_dir: &Path, descriptor: &MlArtifactDescriptor) -> Result<(), String> {
    let path = run_dir.join(&descriptor.file_name);
    let metadata =
        fs::metadata(&path).map_err(|_| "ML worker 결과 아티팩트를 찾지 못했습니다.".to_owned())?;
    if !metadata.is_file() || metadata.len() != descriptor.byte_size {
        return Err("ML worker 아티팩트 크기가 결과 계약과 다릅니다.".to_owned());
    }
    let bytes = fs::read(path).map_err(|_| "ML worker 아티팩트를 읽지 못했습니다.".to_owned())?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != descriptor.sha256 {
        return Err("ML worker 아티팩트 해시가 결과 계약과 다릅니다.".to_owned());
    }
    Ok(())
}

fn parse_result_request(bytes: &[u8]) -> Result<MlTrainingJobCompleteRequest, String> {
    serde_json::from_slice(bytes)
        .map_err(|_| "ML worker 결과 JSON 계약이 올바르지 않습니다.".to_owned())
}

fn write_new_synced(path: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| format!("{label} 파일을 만들지 못했습니다."))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| format!("{label} 파일을 안전하게 저장하지 못했습니다."))
}

fn stage_worker_input(
    bridge: &PersistenceBridge,
    source: MlTrainingJobSource,
    run_dir: &Path,
) -> Result<(Vec<u8>, Option<PathBuf>), String> {
    match source {
        MlTrainingJobSource::Manifest(bundle) => serde_json::to_vec(&bundle)
            .map(|bytes| (bytes, None))
            .map_err(|_| "ML worker bundle을 직렬화하지 못했습니다.".to_owned()),
        MlTrainingJobSource::ShardSet { job, shard_set } => {
            let shard_dir = run_dir.join("shards");
            fs::create_dir(&shard_dir)
                .map_err(|_| "ML worker shard 폴더를 만들지 못했습니다.".to_owned())?;
            let mut files = Vec::with_capacity(shard_set.shards.len());
            for (index, descriptor) in shard_set.shards.iter().enumerate() {
                let (manifest, payload_json) = shard_payload(bridge, &descriptor.manifest_id)?;
                if manifest.content_sha256 != descriptor.content_sha256
                    || manifest.feature_schema_sha256 != shard_set.feature_schema_sha256
                {
                    return Err("ML worker shard 계보가 준비된 집합과 다릅니다.".to_owned());
                }
                let file_name = format!("shard-{index:04}.json");
                let path = shard_dir.join(&file_name);
                write_new_synced(&path, payload_json.as_bytes(), "ML worker shard")?;
                files.push(MlShardWorkerFile {
                    manifest,
                    file_name,
                    byte_size: u64::try_from(payload_json.len())
                        .map_err(|_| "ML worker shard 크기가 범위를 초과했습니다.".to_owned())?,
                });
            }
            let bundle = MlShardWorkerBundle {
                contract_version: job.contract_version.clone(),
                job,
                shard_set,
                dataset_shards: files,
                live_order_allowed: false,
            };
            serde_json::to_vec(&bundle)
                .map(|bytes| (bytes, Some(shard_dir)))
                .map_err(|_| "ML shard worker bundle을 직렬화하지 못했습니다.".to_owned())
        }
    }
}

fn run_job(
    app: &AppHandle,
    bridge: &PersistenceBridge,
    job_id: &str,
) -> Result<MlWorkerRunResult, String> {
    let source = training_job_source(bridge, job_id)?;
    let job = source.job().clone();
    let _running = RunningJobGuard::claim(&job.job_id)?;
    let paths = resolve_paths(app)?;
    fs::create_dir_all(&paths.run_root)
        .map_err(|_| "ML worker 실행 루트를 만들지 못했습니다.".to_owned())?;
    let job_root = paths.run_root.join(&job.job_id);
    fs::create_dir_all(&job_root)
        .map_err(|_| "ML worker 작업 폴더를 만들지 못했습니다.".to_owned())?;
    let attempt_started_at_ms = persistence::now_ms()?;
    let run_dir = job_root.join(format!("attempt-{attempt_started_at_ms}"));
    fs::create_dir(&run_dir)
        .map_err(|_| "ML worker 실행 시도 폴더를 만들지 못했습니다.".to_owned())?;
    let input_path = run_dir.join("input.json");
    let (bundle_bytes, staged_shards) = match stage_worker_input(bridge, source, &run_dir) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_dir);
            return Err(error);
        }
    };
    write_new_synced(&input_path, &bundle_bytes, "ML worker 입력")?;
    let args = [
        paths.script.as_os_str().to_os_string(),
        OsString::from("--input"),
        input_path.as_os_str().to_os_string(),
        OsString::from("--output-dir"),
        run_dir.as_os_str().to_os_string(),
    ];
    let outcome = match run_process(
        &paths.python,
        &args,
        &run_dir,
        Duration::from_secs(u64::from(job.timeout_seconds)),
        job.memory_limit_mb,
        job.max_threads,
    ) {
        Ok(outcome) => outcome,
        Err(_) => {
            let _ = fs::remove_file(&input_path);
            if let Some(shard_dir) = &staged_shards {
                let _ = fs::remove_dir_all(shard_dir);
            }
            let completion = fail_job(
                bridge,
                &job.job_id,
                &job.input_sha256,
                "worker_launch_failure",
                persistence::now_ms()?,
            )?;
            return Ok(MlWorkerRunResult {
                completion,
                elapsed_ms: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
                memory_limit_enforced: false,
                live_order_allowed: false,
            });
        }
    };
    let _ = fs::remove_file(&input_path);
    if let Some(shard_dir) = &staged_shards {
        let _ = fs::remove_dir_all(shard_dir);
    }
    let completed_at_ms = persistence::now_ms()?;
    let failure_code = if outcome.timed_out {
        Some("worker_timeout")
    } else if outcome.stdout.exceeded {
        Some("worker_stdout_limit")
    } else if outcome.stderr.exceeded {
        Some("worker_stderr_limit")
    } else if !outcome.success {
        Some("worker_process_failure")
    } else {
        None
    };
    if let Some(code) = failure_code {
        let completion = fail_job(
            bridge,
            &job.job_id,
            &job.input_sha256,
            code,
            completed_at_ms,
        )?;
        return Ok(MlWorkerRunResult {
            completion,
            elapsed_ms: outcome.elapsed_ms,
            stdout_bytes: outcome.stdout.bytes.len(),
            stderr_bytes: outcome.stderr.bytes.len(),
            memory_limit_enforced: outcome.memory_limit_enforced,
            live_order_allowed: false,
        });
    }
    let result_path = run_dir.join(format!("{}.result.json", job.job_id));
    let result_metadata = match fs::metadata(&result_path) {
        Ok(metadata) => metadata,
        Err(_) => {
            let completion = fail_job(
                bridge,
                &job.job_id,
                &job.input_sha256,
                "worker_missing_result",
                completed_at_ms,
            )?;
            return Ok(MlWorkerRunResult {
                completion,
                elapsed_ms: outcome.elapsed_ms,
                stdout_bytes: outcome.stdout.bytes.len(),
                stderr_bytes: outcome.stderr.bytes.len(),
                memory_limit_enforced: outcome.memory_limit_enforced,
                live_order_allowed: false,
            });
        }
    };
    if !result_metadata.is_file() || result_metadata.len() > RESULT_LIMIT_BYTES {
        let completion = fail_job(
            bridge,
            &job.job_id,
            &job.input_sha256,
            "worker_result_limit",
            completed_at_ms,
        )?;
        return Ok(MlWorkerRunResult {
            completion,
            elapsed_ms: outcome.elapsed_ms,
            stdout_bytes: outcome.stdout.bytes.len(),
            stderr_bytes: outcome.stderr.bytes.len(),
            memory_limit_enforced: outcome.memory_limit_enforced,
            live_order_allowed: false,
        });
    }
    let result_bytes =
        fs::read(&result_path).map_err(|_| "ML worker 결과 JSON을 읽지 못했습니다.".to_owned())?;
    let request: MlTrainingJobCompleteRequest = match parse_result_request(&result_bytes) {
        Ok(request) => request,
        Err(_) => {
            let completion = fail_job(
                bridge,
                &job.job_id,
                &job.input_sha256,
                "worker_invalid_result",
                completed_at_ms,
            )?;
            return Ok(MlWorkerRunResult {
                completion,
                elapsed_ms: outcome.elapsed_ms,
                stdout_bytes: outcome.stdout.bytes.len(),
                stderr_bytes: outcome.stderr.bytes.len(),
                memory_limit_enforced: outcome.memory_limit_enforced,
                live_order_allowed: false,
            });
        }
    };
    if request
        .artifact
        .as_ref()
        .is_some_and(|artifact| verify_artifact(&run_dir, artifact).is_err())
    {
        let completion = fail_job(
            bridge,
            &job.job_id,
            &job.input_sha256,
            "worker_artifact_invalid",
            completed_at_ms,
        )?;
        return Ok(MlWorkerRunResult {
            completion,
            elapsed_ms: outcome.elapsed_ms,
            stdout_bytes: outcome.stdout.bytes.len(),
            stderr_bytes: outcome.stderr.bytes.len(),
            memory_limit_enforced: outcome.memory_limit_enforced,
            live_order_allowed: false,
        });
    }
    let completion = complete_training_job(bridge, request)?;
    Ok(MlWorkerRunResult {
        completion,
        elapsed_ms: outcome.elapsed_ms,
        stdout_bytes: outcome.stdout.bytes.len(),
        stderr_bytes: outcome.stderr.bytes.len(),
        memory_limit_enforced: outcome.memory_limit_enforced,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub async fn ml_training_job_run(
    app: AppHandle,
    job_id: String,
) -> Result<MlWorkerRunResult, String> {
    let app_for_worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bridge = app_for_worker.state::<PersistenceBridge>();
        run_job(&app_for_worker, &bridge, &job_id)
    })
    .await
    .map_err(|_| "ML worker 실행 스레드가 종료되었습니다.".to_owned())?
}

#[cfg(windows)]
mod windows_job {
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, process::Child};

    type Handle = *mut c_void;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x0000_0200;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic_limit_information: BasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: i32,
            information: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    pub(super) struct WindowsJob {
        handle: Handle,
    }

    impl WindowsJob {
        pub(super) fn assign(child: &Child, memory_limit_mb: u32) -> Result<Self, String> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err("Windows ML Job Object를 만들지 못했습니다.".to_owned());
            }
            let memory_bytes = usize::try_from(memory_limit_mb)
                .ok()
                .and_then(|value| value.checked_mul(1024 * 1024))
                .ok_or_else(|| "ML worker 메모리 상한을 계산하지 못했습니다.".to_owned())?;
            let mut information = ExtendedLimitInformation::default();
            information.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            information.process_memory_limit = memory_bytes;
            information.job_memory_limit = memory_bytes;
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&information as *const ExtendedLimitInformation).cast(),
                    u32::try_from(size_of::<ExtendedLimitInformation>()).unwrap_or(u32::MAX),
                )
            };
            let assigned = if configured != 0 {
                unsafe { AssignProcessToJobObject(handle, child.as_raw_handle().cast()) }
            } else {
                0
            };
            if configured == 0 || assigned == 0 {
                unsafe { CloseHandle(handle) };
                return Err("Windows ML worker 메모리 Job Object 적용에 실패했습니다.".to_owned());
            }
            Ok(Self { handle })
        }

        pub(super) fn is_enforced(&self) -> bool {
            !self.handle.is_null()
        }
    }

    impl Drop for WindowsJob {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe { CloseHandle(self.handle) };
                self.handle = std::ptr::null_mut();
            }
        }
    }
}

#[cfg(windows)]
use windows_job::WindowsJob;

#[cfg(not(windows))]
struct WindowsJob;

#[cfg(not(windows))]
impl WindowsJob {
    fn assign(_child: &Child, _memory_limit_mb: u32) -> Result<Self, String> {
        Err("이 빌드는 아직 ML worker 메모리 상한을 지원하지 않습니다.".to_owned())
    }

    fn is_enforced(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn capped_reader_keeps_prefix_and_reports_overflow() {
        let output = read_capped(Cursor::new(vec![7_u8; 32]), 10);
        assert_eq!(output.bytes, vec![7_u8; 10]);
        assert!(output.exceeded);
    }

    #[test]
    fn worker_environment_does_not_inherit_secret_names() {
        let root = std::env::temp_dir();
        let values = allowed_environment_values(&root, 3);
        let names = values
            .iter()
            .map(|(name, _)| name.to_string_lossy().to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert!(names.iter().all(|name| !name.contains("token")));
        assert!(names.iter().all(|name| !name.contains("secret")));
        assert!(names.iter().any(|name| name == "pythonnousersite"));
        assert!(names.iter().any(|name| name == "omp_num_threads"));
    }

    #[test]
    fn artifact_verification_rejects_tampering() {
        let root =
            std::env::temp_dir().join(format!("investa-ml-artifact-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("model.txt"), b"model-v1").expect("write");
        let descriptor = MlArtifactDescriptor {
            file_name: "model.txt".to_owned(),
            format: crate::ml_pipeline::MlArtifactFormat::LightgbmText,
            sha256: format!("{:x}", Sha256::digest(b"different")),
            byte_size: 8,
        };
        assert!(verify_artifact(&root, &descriptor).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn invalid_result_json_is_rejected() {
        assert!(parse_result_request(br#"{"succeeded":true}"#).is_err());
        assert!(parse_result_request(b"not-json").is_err());
    }

    #[test]
    fn duplicate_worker_job_is_rejected_until_guard_drops() {
        let job_id = format!("runner-claim-{}", std::process::id());
        let first = RunningJobGuard::claim(&job_id).expect("first claim");
        assert!(RunningJobGuard::claim(&job_id).is_err());
        drop(first);
        assert!(RunningJobGuard::claim(&job_id).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_object_runs_and_reports_abnormal_exit() {
        let root =
            std::env::temp_dir().join(format!("investa-ml-process-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        let command = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32")
            .join("cmd.exe");
        let success = run_process(
            &command,
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from("exit 0"),
            ],
            &root,
            Duration::from_secs(2),
            512,
            1,
        )
        .expect("success process");
        assert!(success.success);
        assert!(success.memory_limit_enforced);
        let failed = run_process(
            &command,
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from("exit 7"),
            ],
            &root,
            Duration::from_secs(2),
            512,
            1,
        )
        .expect("failed process");
        assert!(!failed.success);
        assert!(!failed.timed_out);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_object_kills_worker_on_timeout() {
        let root =
            std::env::temp_dir().join(format!("investa-ml-timeout-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        let ping = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32")
            .join("ping.exe");
        let outcome = run_process(
            &ping,
            &[
                OsString::from("-n"),
                OsString::from("6"),
                OsString::from("127.0.0.1"),
            ],
            &root,
            Duration::from_millis(50),
            512,
            1,
        )
        .expect("timeout process");
        assert!(outcome.timed_out);
        assert!(!outcome.success);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
