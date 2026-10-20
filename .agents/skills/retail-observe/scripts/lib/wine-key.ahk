#Requires AutoHotkey v2.0

if A_Args.Length != 3
    ExitApp(2)

title := A_Args[1]
key := A_Args[2]
holdMs := A_Args[3]
MAX_HOLD_MS := 60000

if title = "" || !RegExMatch(key, "^0x[0-9A-Fa-f]+$") || !RegExMatch(holdMs, "^[0-9]+$") || holdMs > MAX_HOLD_MS
    ExitApp(2)

SetTitleMatchMode(2)
windows := WinGetList(title)
if windows.Length != 1
    ExitApp(3)

window := windows[1]
target := "ahk_id " window
WinActivate(target)
if !WinWaitActive(target, , 3) || WinActive("A") != window
    ExitApp(4)

SendEvent("{vk" SubStr(key, 3) " down}")
Sleep(holdMs)
SendEvent("{vk" SubStr(key, 3) " up}")
