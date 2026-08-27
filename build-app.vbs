Option Explicit

Dim shell, fileSystem, projectRoot, powershellPath, runnerPath, logPath, command, exitCode

Set shell = CreateObject("WScript.Shell")
Set fileSystem = CreateObject("Scripting.FileSystemObject")
projectRoot = fileSystem.GetParentFolderName(WScript.ScriptFullName)
powershellPath = shell.ExpandEnvironmentStrings("%SystemRoot%") & _
  "\System32\WindowsPowerShell\v1.0\powershell.exe"
runnerPath = fileSystem.BuildPath(projectRoot, "tools\start_app_hidden.ps1")
logPath = fileSystem.BuildPath(projectRoot, ".runtime\app-launch.log")

command = Quote(powershellPath) & _
  " -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass" & _
  " -WindowStyle Hidden -File " & Quote(runnerPath) & " -Rebuild"
exitCode = shell.Run(command, 0, True)

If exitCode <> 0 Then
  MsgBox "Atsumi Next could not be rebuilt." & vbCrLf & vbCrLf & _
    "Details were saved here:" & vbCrLf & logPath, _
    vbCritical + vbOKOnly, "Atsumi Next"
End If

WScript.Quit exitCode

Function Quote(value)
  Quote = Chr(34) & Replace(value, Chr(34), Chr(34) & Chr(34)) & Chr(34)
End Function
