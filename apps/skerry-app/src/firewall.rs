//! Windows Firewall: check that other computers may connect to Skerry, and
//! add a rule that allows it. Other systems report "n/a".
//!
//! When Windows first asks whether Skerry may accept connections and the
//! prompt is dismissed (or answered for the wrong network type), Windows adds
//! rules that block Skerry. Then this computer can connect to others but no
//! one can connect to it, which is easy to miss.

#[cfg(not(target_os = "windows"))]
pub fn status(_port: u16) -> String {
    "n/a".into()
}

#[cfg(not(target_os = "windows"))]
pub fn allow() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn has_rules() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn remove_rules() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub use imp::{allow, has_rules, remove_rules, status};

#[cfg(target_os = "windows")]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Output};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// ERROR_CANCELLED: the user declined the administrator prompt.
    const DECLINED: i32 = 1223;

    /// Inbound rules for the program in `$p`, however its path was written.
    const MATCH_RULES: &str = "$rules = @(Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | \
        Where-Object { $_.Program -and ([Environment]::ExpandEnvironmentVariables($_.Program) -ieq $p) } | \
        Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object { $_.Direction -eq 'Inbound' })";

    /// Every rule for the program in `$p`, plus the one named Skerry.
    const ALL_RULES: &str = "$rules = @(Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | \
        Where-Object { $_.Program -and ([Environment]::ExpandEnvironmentVariables($_.Program) -ieq $p) } | \
        Get-NetFirewallRule -ErrorAction SilentlyContinue) + @(Get-NetFirewallRule -Name 'Skerry' -ErrorAction SilentlyContinue)";

    fn exe() -> Result<String, String> {
        std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
    }

    fn quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }

    fn base64(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        out
    }

    /// PowerShell's -EncodedCommand takes base64 of UTF-16LE text, which
    /// sidesteps quoting problems with paths.
    fn encode(script: &str) -> String {
        let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        base64(&bytes)
    }

    fn powershell(script: &str) -> std::io::Result<Output> {
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encode(script)])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    }

    /// "ok", "blocked" (a rule blocks Skerry), "missing" (nothing allows it,
    /// so Windows will block or ask) or "unknown".
    pub fn status(port: u16) -> String {
        let Ok(exe) = exe() else { return "unknown".into() };
        let script = format!(
            "$p = {path}\n\
             if (-not (Get-NetFirewallProfile -ErrorAction SilentlyContinue | Where-Object {{ $_.Enabled -eq 'True' }})) {{ 'ok'; exit }}\n\
             {MATCH_RULES}\n\
             $on = @($rules | Where-Object {{ $_.Enabled -eq 'True' }})\n\
             if ($on | Where-Object {{ $_.Action -eq 'Block' }}) {{ 'blocked'; exit }}\n\
             if ($on | Where-Object {{ $_.Action -eq 'Allow' }}) {{ 'ok'; exit }}\n\
             $port = @(Get-NetFirewallPortFilter -Protocol TCP -ErrorAction SilentlyContinue | \
                 Where-Object {{ $_.LocalPort -contains '{port}' }} | Get-NetFirewallRule -ErrorAction SilentlyContinue | \
                 Where-Object {{ $_.Direction -eq 'Inbound' -and $_.Enabled -eq 'True' -and $_.Action -eq 'Allow' }})\n\
             if ($port) {{ 'ok' }} else {{ 'missing' }}",
            path = quote(&exe)
        );
        match powershell(&script) {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                match text.lines().map(str::trim).rfind(|l| !l.is_empty()) {
                    Some(s @ ("ok" | "blocked" | "missing")) => s.to_string(),
                    _ => {
                        tracing::info!("firewall check: {}", String::from_utf8_lossy(&out.stderr).trim());
                        "unknown".into()
                    }
                }
            }
            Err(e) => {
                tracing::info!("firewall check failed: {e}");
                "unknown".into()
            }
        }
    }

    /// Whether any firewall rule belongs to Skerry.
    pub fn has_rules() -> bool {
        let Ok(exe) = exe() else { return false };
        let script = format!("$p = {}\n{ALL_RULES}\nif ($rules.Count -gt 0) {{ 'yes' }} else {{ 'no' }}", quote(&exe));
        powershell(&script).is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("yes"))
    }

    /// Delete every firewall rule that belongs to Skerry (for uninstalling).
    /// Shows the Windows administrator prompt.
    pub fn remove_rules() -> Result<(), String> {
        let exe = exe()?;
        let inner = format!(
            "$ErrorActionPreference = 'Stop'\n$p = {}\n{ALL_RULES}\n$rules | Remove-NetFirewallRule -ErrorAction SilentlyContinue",
            quote(&exe)
        );
        run_elevated(&inner).map_err(|e| format!("{e} Nothing was removed."))?;
        tracing::info!("removed Skerry's Windows Firewall rules");
        Ok(())
    }

    /// Replace Skerry's inbound rules with one that allows it on every
    /// network type. Shows the Windows administrator prompt.
    pub fn allow() -> Result<(), String> {
        let exe = exe()?;
        let inner = format!(
            "$ErrorActionPreference = 'Stop'\n\
             $p = {path}\n\
             Remove-NetFirewallRule -Name 'Skerry' -ErrorAction SilentlyContinue\n\
             {MATCH_RULES}\n\
             $rules | Remove-NetFirewallRule -ErrorAction SilentlyContinue\n\
             New-NetFirewallRule -Name 'Skerry' -DisplayName 'Skerry' \
                 -Description 'Lets other computers running Skerry connect to this one.' \
                 -Direction Inbound -Action Allow -Program $p -Profile Any | Out-Null",
            path = quote(&exe)
        );
        run_elevated(&inner)?;
        tracing::info!("added a Windows Firewall rule for Skerry");
        Ok(())
    }

    /// Run a PowerShell script as administrator (Windows asks the user).
    fn run_elevated(inner: &str) -> Result<(), String> {
        let outer = format!(
            "try {{ $proc = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
             -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand','{}'; exit $proc.ExitCode }} \
             catch {{ exit {DECLINED} }}",
            encode(inner)
        );
        let out = powershell(&outer).map_err(|e| e.to_string())?;
        match out.status.code() {
            Some(0) => Ok(()),
            Some(DECLINED) => Err("The change needs administrator approval, which was declined.".into()),
            code => Err(format!(
                "Couldn't change the firewall (exit code {code:?}). {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn base64_matches_rfc4648() {
            assert_eq!(super::base64(b""), "");
            assert_eq!(super::base64(b"f"), "Zg==");
            assert_eq!(super::base64(b"fo"), "Zm8=");
            assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
        }
    }
}
