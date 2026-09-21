use crate::runtime_policy::normalize_windows_path;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningProcessInfo {
    pub pid: u32,
    pub name: String,
    pub exe_name: String,
    pub title: String,
    pub path: String,
}

struct EnumContext {
    seen_paths: HashSet<String>,
    processes: Vec<RunningProcessInfo>,
}

struct OwnedProcessHandle(HANDLE);

impl OwnedProcessHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle)
    }
}

impl Drop for OwnedProcessHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn query_process_path(process: &OwnedProcessHandle) -> Option<String> {
    let mut path_buf = vec![0u16; 32_768];
    let mut size = path_buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path_buf.as_mut_ptr()),
            &mut size,
        )
    }
    .ok()?;
    Some(String::from_utf16_lossy(&path_buf[..size as usize]))
}

// Taking ownership here keeps the query, duplicate and invalid-path exits under
// the same Drop scope. Tests supply counted handles without opening a process.
fn collect_window_process<H>(
    ctx: &mut EnumContext,
    pid: u32,
    title: &str,
    process: H,
    query: impl FnOnce(&H) -> Option<String>,
) {
    let Some(full_path) = query(&process) else {
        return;
    };
    let Some(identity) = normalize_windows_path(&full_path) else {
        return;
    };
    let exe_name = Path::new(&full_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if exe_name.is_empty() || !ctx.seen_paths.insert(identity) {
        return;
    }
    let stem = Path::new(&exe_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| exe_name.clone());
    ctx.processes.push(RunningProcessInfo {
        pid,
        name: stem,
        exe_name: exe_name.to_lowercase(),
        title: title.to_string(),
        path: full_path,
    });
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumContext);

    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let length = GetWindowTextLengthW(hwnd);
    if length == 0 {
        return BOOL(1);
    }

    let mut title_buf = vec![0u16; (length + 1) as usize];
    let len = GetWindowTextW(hwnd, &mut title_buf);
    if len == 0 {
        return BOOL(1);
    }

    let title = String::from_utf16_lossy(&title_buf[..len as usize]);
    let title_trim = title.trim();
    if title_trim.is_empty() || title_trim == "Program Manager" {
        return BOOL(1);
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 {
        return BOOL(1);
    }

    if let Ok(process) = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
    }.map(OwnedProcessHandle::new) {
        collect_window_process(ctx, pid, title_trim, process, query_process_path);
    }

    BOOL(1)
}

pub fn get_running_processes() -> Vec<RunningProcessInfo> {
    let mut ctx = EnumContext {
        seen_paths: HashSet::new(),
        processes: Vec::new(),
    };

    unsafe {
        let lparam = LPARAM(&mut ctx as *mut _ as isize);
        let _ = EnumWindows(Some(enum_windows_proc), lparam);
    }

    ctx.processes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    ctx.processes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct CountedHandle(Rc<Cell<usize>>);

    impl Drop for CountedHandle {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn context() -> EnumContext {
        EnumContext { seen_paths: HashSet::new(), processes: Vec::new() }
    }

    #[test]
    fn successful_and_duplicate_windows_close_every_handle() {
        let closed = Rc::new(Cell::new(0));
        let mut ctx = context();
        for (index, path) in [r"C:\Games\Game.exe", r"c:\games\GAME.EXE"].iter().enumerate() {
            let title = if index == 0 { "first title" } else { "duplicate title" };
            collect_window_process(&mut ctx, 42 + index as u32, title,
                CountedHandle(closed.clone()), |_| Some((*path).into()));
            assert_eq!(closed.get(), index + 1);
        }
        assert_eq!(ctx.processes.len(), 1);
        assert_eq!(ctx.processes[0].pid, 42);
        assert_eq!(ctx.processes[0].title, "first title");
        assert_eq!(ctx.processes[0].path, r"C:\Games\Game.exe");
    }

    #[test]
    fn failed_and_empty_queries_close_every_handle() {
        let closed = Rc::new(Cell::new(0));
        let mut ctx = context();
        for (index, result) in [None, Some(String::new())].into_iter().enumerate() {
            collect_window_process(&mut ctx, 42, "title",
                CountedHandle(closed.clone()), |_| result);
            assert_eq!(closed.get(), index + 1);
        }
        assert!(ctx.processes.is_empty());
    }

    #[test]
    fn same_basename_different_paths_keep_distinct_process_rows() {
        let closed = Rc::new(Cell::new(0));
        let mut ctx = context();
        for (index, path) in [r"C:\Games\game.exe", r"D:\Games\game.exe"].iter().enumerate() {
            collect_window_process(&mut ctx, 42 + index as u32, "title",
                CountedHandle(closed.clone()), |_| Some((*path).into()));
        }
        assert_eq!(closed.get(), 2);
        assert_eq!(ctx.processes.len(), 2);
        assert_eq!(ctx.processes[0].exe_name, ctx.processes[1].exe_name);
        assert_ne!(ctx.processes[0].path, ctx.processes[1].path);
        assert_ne!(ctx.processes[0].pid, ctx.processes[1].pid);
    }

    #[test]
    fn runtime_equivalent_paths_collapse_without_changing_the_first_dto() {
        for paths in [
            vec![
                r"\\?\C:\Games\.\Game.exe",
                r"C:\Games\Game.exe",
                r"c:\games\folder\..\GAME.EXE",
                r"C:/Games/./Game.exe",
            ],
            vec![
                r"\\?\UNC\Server\Share\Games\Game.exe",
                r"\\server\share\games\.\game.exe",
                r"//SERVER/share/games/folder/../Game.exe",
            ],
        ] {
            let closed = Rc::new(Cell::new(0));
            let mut ctx = context();
            for (index, path) in paths.iter().enumerate() {
                let title = if index == 0 { "first title" } else { "duplicate title" };
                collect_window_process(&mut ctx, 42 + index as u32, title,
                    CountedHandle(closed.clone()), |_| Some((*path).into()));
                assert_eq!(closed.get(), index + 1);
            }
            assert_eq!(ctx.processes.len(), 1);
            assert_eq!(ctx.processes[0].path, paths[0]);
            assert_eq!(ctx.processes[0].pid, 42);
            assert_eq!(ctx.processes[0].title, "first title");
            assert_eq!(ctx.processes[0].exe_name, "game.exe");
        }
    }

    #[test]
    fn malformed_paths_are_rejected_and_every_handle_is_closed() {
        let closed = Rc::new(Cell::new(0));
        let mut ctx = context();
        for (index, path) in [
            r"game.exe",
            r"C:game.exe",
            r"C:\..\game.exe",
            r"C:\Games\game.exe:stream",
            r"\\server\share",
            r"\\server\share\..\game.exe",
            r"C:\Games\game?.exe",
            "C:\\Games\\game\0.exe",
        ].iter().enumerate() {
            collect_window_process(&mut ctx, 42, "title",
                CountedHandle(closed.clone()), |_| Some((*path).into()));
            assert_eq!(closed.get(), index + 1);
        }
        assert!(ctx.processes.is_empty());
        assert!(ctx.seen_paths.is_empty());
    }
}
