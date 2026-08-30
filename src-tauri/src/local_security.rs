use std::path::Path;

#[cfg(windows)]
use std::process::Command;

fn has_codex_deny_acl(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.contains("CodexSandboxUsers:") && line.contains("(N)"))
}

#[cfg(windows)]
pub fn harden_app_data(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|_| "Investa 앱 데이터 폴더를 만들지 못했습니다.".to_owned())?;
    let group = Command::new("net.exe")
        .args(["localgroup", "CodexSandboxUsers"])
        .output();
    if !group.is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    let current = Command::new("icacls.exe")
        .arg(path)
        .output()
        .map_err(|_| "Investa 로컬 데이터 접근통제를 확인하지 못했습니다.".to_owned())?;
    let current = String::from_utf8_lossy(&current.stdout);
    if has_codex_deny_acl(&current) {
        return Ok(());
    }
    let status = Command::new("icacls.exe")
        .arg(path)
        .args(["/deny", "CodexSandboxUsers:(OI)(CI)(F)"])
        .status()
        .map_err(|_| "Investa 로컬 데이터 접근통제를 적용하지 못했습니다.".to_owned())?;
    if !status.success() {
        return Err("Investa 로컬 데이터에서 Codex sandbox 접근을 차단하지 못했습니다.".to_owned());
    }
    Ok(())
}

#[cfg(unix)]
pub fn harden_app_data(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path)
        .map_err(|_| "Investa 앱 데이터 폴더를 만들지 못했습니다.".to_owned())?;
    let mut permissions = std::fs::metadata(path)
        .map_err(|_| "Investa 앱 데이터 폴더 권한을 읽지 못했습니다.".to_owned())?
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).map_err(|_| {
        "Investa 앱 데이터 폴더를 현재 사용자 전용으로 제한하지 못했습니다.".to_owned()
    })
}

#[cfg(not(any(windows, unix)))]
pub fn harden_app_data(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|_| "Investa 앱 데이터 폴더를 만들지 못했습니다.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_an_explicit_codex_no_access_entry() {
        assert!(has_codex_deny_acl(
            r"C:\data DESKTOP\CodexSandboxUsers:(OI)(CI)(N)"
        ));
        assert!(!has_codex_deny_acl(
            r"C:\data DESKTOP\CodexSandboxUsers:(I)(OI)(CI)(RX)"
        ));
        assert!(!has_codex_deny_acl(
            r"C:\data DESKTOP\OtherSandboxUsers:(OI)(CI)(N)"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hardens_app_data_to_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let path =
            std::env::temp_dir().join(format!("investa-local-security-{}", std::process::id()));
        harden_app_data(&path).expect("harden app data");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::remove_dir(path).expect("remove test directory");
    }
}
