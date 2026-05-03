use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// The standalone abtop statusline script.
/// Reads rate limits from Claude Code statusLine JSON.
/// When Anthropic rate_limits are absent (e.g. GLM proxy), falls back to
/// the GLM quota API with a 3-minute cache throttle.
const STATUSLINE_SCRIPT: &str = r#"#!/bin/bash
# abtop StatusLine hook — writes rate limit data for abtop to read.
# Installed by: abtop --setup
INPUT=""
while IFS= read -r -t 5 line || [ -n "$line" ]; do
    INPUT="${INPUT}${line}
"
done
[ -z "$INPUT" ] && exit 0

printf '%s' "$INPUT" | python3 -c "
import sys, json, time, os, urllib.request, urllib.parse

config_dir = os.environ.get('CLAUDE_CONFIG_DIR', os.path.join(os.path.expanduser('~'), '.claude'))
out_path = os.path.join(config_dir, 'abtop-rate-limits.json')

data = json.load(sys.stdin)
rl = data.get('rate_limits')
out = {'source': 'claude', 'updated_at': int(time.time())}

# Source 1: Anthropic rate_limits from statusLine JSON
if rl:
    fh = rl.get('five_hour')
    if fh:
        out['five_hour'] = {'used_percentage': fh.get('used_percentage', 0), 'resets_at': fh.get('resets_at', 0)}
    sd = rl.get('seven_day')
    if sd:
        out['seven_day'] = {'used_percentage': sd.get('used_percentage', 0), 'resets_at': sd.get('resets_at', 0)}

# Source 2: GLM quota API (throttled: once per 3 minutes)
need_glm = ('five_hour' not in out or 'seven_day' not in out)
if need_glm:
    now = time.time()
    cache_file = os.path.join(config_dir, 'abtop-quota-cache.json')
    try:
        with open(cache_file) as f:
            cache = json.load(f)
        if now - cache.get('ts', 0) < 180:
            need_glm = False
            if 'five_hour' not in out and 'five_hour' in cache:
                out['five_hour'] = cache['five_hour']
            if 'seven_day' not in out and 'seven_day' in cache:
                out['seven_day'] = cache['seven_day']
    except Exception:
        pass

if need_glm:
    base_url = os.environ.get('ANTHROPIC_BASE_URL', '')
    auth_token = os.environ.get('ANTHROPIC_AUTH_TOKEN', '')
    if base_url and auth_token:
        try:
            parsed = urllib.parse.urlparse(base_url)
            domain = f'{parsed.scheme}://{parsed.netloc}'
            quota_url = f'{domain}/api/monitor/usage/quota/limit'
            req = urllib.request.Request(quota_url, headers={
                'Authorization': auth_token,
                'Content-Type': 'application/json'
            })
            with urllib.request.urlopen(req, timeout=5) as resp:
                quota = json.loads(resp.read())
            limits = quota.get('data', {}).get('limits', [])
            cache = {'ts': now}
            for item in limits:
                if item.get('type') == 'TOKENS_LIMIT':
                    pct = item.get('percentage', 0)
                    reset_ts = item.get('nextResetTime', 0)
                    unit = item.get('unit')
                    if unit == 3:
                        out['five_hour'] = {'used_percentage': pct, 'resets_at': reset_ts // 1000}
                        cache['five_hour'] = out['five_hour']
                    elif unit == 6:
                        out['seven_day'] = {'used_percentage': pct, 'resets_at': reset_ts // 1000}
                        cache['seven_day'] = out['seven_day']
            tmp = cache_file + '.tmp'
            with open(tmp, 'w') as f:
                json.dump(cache, f)
            os.replace(tmp, cache_file)
        except Exception:
            pass

if 'five_hour' in out or 'seven_day' in out:
    tmp = out_path + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(out, f)
    os.replace(tmp, out_path)
" 2>/dev/null
"#;

/// Generate a combined statusline script that runs both the existing command
/// and abtop's rate-limit extraction. Used when another tool (e.g. claude-hud)
/// already occupies the statusLine.
fn generate_combined_script(abtop_script: &str, existing_command: &str) -> String {
    format!(r#"#!/bin/bash
# abtop combined StatusLine hook — runs existing command + abtop rate-limit extraction.
# Installed by: abtop --setup
INPUT=""
while IFS= read -r -t 5 line || [ -n "$line" ]; do
    INPUT="${{INPUT}}${{line}}
"
done
[ -z "$INPUT" ] && exit 0

# --- abtop rate-limit extraction (background) ---
printf '%s' "$INPUT" | python3 -c "
import sys, json, time, os, urllib.request, urllib.parse

config_dir = os.environ.get('CLAUDE_CONFIG_DIR', os.path.join(os.path.expanduser('~'), '.claude'))
out_path = os.path.join(config_dir, 'abtop-rate-limits.json')

data = json.load(sys.stdin)
rl = data.get('rate_limits')
out = {{'source': 'claude', 'updated_at': int(time.time())}}

if rl:
    fh = rl.get('five_hour')
    if fh:
        out['five_hour'] = {{'used_percentage': fh.get('used_percentage', 0), 'resets_at': fh.get('resets_at', 0)}}
    sd = rl.get('seven_day')
    if sd:
        out['seven_day'] = {{'used_percentage': sd.get('used_percentage', 0), 'resets_at': sd.get('resets_at', 0)}}

need_glm = ('five_hour' not in out or 'seven_day' not in out)
if need_glm:
    now = time.time()
    cache_file = os.path.join(config_dir, 'abtop-quota-cache.json')
    try:
        with open(cache_file) as f:
            cache = json.load(f)
        if now - cache.get('ts', 0) < 180:
            need_glm = False
            if 'five_hour' not in out and 'five_hour' in cache:
                out['five_hour'] = cache['five_hour']
            if 'seven_day' not in out and 'seven_day' in cache:
                out['seven_day'] = cache['seven_day']
    except Exception:
        pass

if need_glm:
    base_url = os.environ.get('ANTHROPIC_BASE_URL', '')
    auth_token = os.environ.get('ANTHROPIC_AUTH_TOKEN', '')
    if base_url and auth_token:
        try:
            parsed = urllib.parse.urlparse(base_url)
            domain = f'{{parsed.scheme}}://{{parsed.netloc}}'
            quota_url = f'{{domain}}/api/monitor/usage/quota/limit'
            req = urllib.request.Request(quota_url, headers={{
                'Authorization': auth_token,
                'Content-Type': 'application/json'
            }})
            with urllib.request.urlopen(req, timeout=5) as resp:
                quota = json.loads(resp.read())
            limits = quota.get('data', {{}}).get('limits', [])
            cache = {{'ts': now}}
            for item in limits:
                if item.get('type') == 'TOKENS_LIMIT':
                    pct = item.get('percentage', 0)
                    reset_ts = item.get('nextResetTime', 0)
                    unit = item.get('unit')
                    if unit == 3:
                        out['five_hour'] = {{'used_percentage': pct, 'resets_at': reset_ts // 1000}}
                        cache['five_hour'] = out['five_hour']
                    elif unit == 6:
                        out['seven_day'] = {{'used_percentage': pct, 'resets_at': reset_ts // 1000}}
                        cache['seven_day'] = out['seven_day']
            tmp = cache_file + '.tmp'
            with open(tmp, 'w') as f:
                json.dump(cache, f)
            os.replace(tmp, cache_file)
        except Exception:
            pass

if 'five_hour' in out or 'seven_day' in out:
    tmp = out_path + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(out, f)
    os.replace(tmp, out_path)
" 2>/dev/null &

# --- existing statusLine command (foreground) ---
printf '%s' "$INPUT" | {existing_command}

wait
"#, existing_command = existing_command)
}

fn claude_dir() -> PathBuf {
    std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

fn script_path() -> PathBuf {
    claude_dir().join("abtop-statusline.sh")
}

fn combined_script_path() -> PathBuf {
    claude_dir().join("abtop-combined-statusline.sh")
}

fn settings_path() -> PathBuf {
    claude_dir().join("settings.json")
}

pub fn run_setup() {
    println!("abtop --setup: configuring Claude Code StatusLine hook\n");

    // Ensure ~/.claude directory exists
    let dir = claude_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("  ✗ failed to create {}: {}", dir.display(), e);
        std::process::exit(1);
    }

    // Step 1: Write the abtop statusline script
    let script = script_path();
    match fs::write(&script, STATUSLINE_SCRIPT) {
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&script, fs::Permissions::from_mode(0o700));
            }
            println!("  ✓ wrote {}", script.display());
        }
        Err(e) => {
            eprintln!("  ✗ failed to write {}: {}", script.display(), e);
            std::process::exit(1);
        }
    }

    // Step 2: Update settings.json
    let settings_file = settings_path();
    let mut settings: Value = if settings_file.exists() {
        let content = match fs::read_to_string(&settings_file) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✗ cannot read {}: {}", settings_file.display(), e);
                std::process::exit(1);
            }
        };
        match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  ✗ {} contains invalid JSON: {}", settings_file.display(), e);
                eprintln!("    fix the file manually before running --setup");
                std::process::exit(1);
            }
        }
    } else {
        Value::Object(Default::default())
    };

    let obj = settings.as_object_mut().unwrap();

    let abtop_cmd = script.display().to_string();

    // Check if statusLine is already configured
    if let Some(existing) = obj.get("statusLine") {
        if let Some(existing_obj) = existing.as_object() {
            if let Some(cmd) = existing_obj.get("command") {
                let cmd_str = cmd.as_str().unwrap_or("").to_string();
                if cmd_str == abtop_cmd || cmd_str.is_empty() {
                    // Already configured by abtop, or empty — just set it
                } else {
                    // Another tool (e.g. claude-hud) owns statusLine.
                    // Generate a combined script that runs both.
                    let combined = combined_script_path();
                    let combined_content = generate_combined_script(&abtop_cmd, &cmd_str);
                    match fs::write(&combined, &combined_content) {
                        Ok(_) => {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = fs::set_permissions(&combined, fs::Permissions::from_mode(0o700));
                            }
                            println!("  ✓ detected existing statusLine: {}", cmd_str);
                            println!("  ✓ wrote combined script: {}", combined.display());
                        }
                        Err(e) => {
                            eprintln!("  ✗ failed to write combined script: {}", e);
                            std::process::exit(1);
                        }
                    }

                    obj.insert(
                        "statusLine".to_string(),
                        serde_json::json!({
                            "type": "command",
                            "command": combined.display().to_string()
                        }),
                    );

                    match fs::write(&settings_file, serde_json::to_string_pretty(&settings).unwrap_or_default()) {
                        Ok(_) => println!("  ✓ updated {} (combined mode)", settings_file.display()),
                        Err(e) => {
                            eprintln!("  ✗ failed to update {}: {}", settings_file.display(), e);
                            std::process::exit(1);
                        }
                    }

                    println!("\n  done! rate limit data will appear in abtop after the next Claude response.");
                    println!("  restart any running Claude Code sessions to activate.");
                    return;
                }
            }
        }
    }

    // Set statusLine config (no existing tool)
    obj.insert(
        "statusLine".to_string(),
        serde_json::json!({
            "type": "command",
            "command": abtop_cmd
        }),
    );

    match fs::write(&settings_file, serde_json::to_string_pretty(&settings).unwrap_or_default()) {
        Ok(_) => println!("  ✓ updated {}", settings_file.display()),
        Err(e) => {
            eprintln!("  ✗ failed to update {}: {}", settings_file.display(), e);
            std::process::exit(1);
        }
    }

    println!("\n  done! rate limit data will appear in abtop after the next Claude response.");
    println!("  restart any running Claude Code sessions to activate.");
}
