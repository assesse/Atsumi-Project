Option Explicit

Dim shell, fileSystem, projectRoot, powershellPath, runnerPath, logPath
Dim command, exitCode, checkOnly

Set shell = CreateObject("WScript.Shell")
Set fileSystem = CreateObject("Scripting.FileSystemObject")

projectRoot = fileSystem.GetParentFolderName(WScript.ScriptFullName)
powershellPath = shell.ExpandEnvironmentStrings("%SystemRoot%") & _
  "\System32\WindowsPowerShell\v1.0\powershell.exe"
runnerPath = fileSystem.BuildPath(projectRoot, "tools\start_app_hidden.ps1")
logPath = fileSystem.BuildPath(projectRoot, ".runtime\app-launch.log")

If Not fileSystem.FileExists(powershellPath) Then
  MsgBox "Windows PowerShell was not found.", vbCritical + vbOKOnly, "Atsumi Next"
  WScript.Quit 1
End If

If Not fileSystem.FileExists(runnerPath) Then
  MsgBox "The Atsumi Next launcher is incomplete:" & vbCrLf & runnerPath, _
    vbCritical + vbOKOnly, "Atsumi Next"
  WScript.Quit 1
End If

checkOnly = False
If WScript.Arguments.Count > 0 Then
  checkOnly = (LCase(WScript.Arguments(0)) = "--check")
End If

command = Quote(powershellPath) & _
  " -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass" & _
  " -WindowStyle Hidden -File " & Quote(runnerPath)

If checkOnly Then
  command = command & " -CheckOnly"
  logPath = fileSystem.BuildPath(projectRoot, ".runtime\launcher-check.log")
End If

exitCode = shell.Run(command, 0, True)

If exitCode = 73 Then
  MsgBox "Atsumi Next is already starting or running.", _
    vbInformation + vbOKOnly, "Atsumi Next"
ElseIf exitCode <> 0 Then
  MsgBox "Atsumi Next could not be started." & vbCrLf & vbCrLf & _
    "Details were saved here:" & vbCrLf & logPath, _
    vbCritical + vbOKOnly, "Atsumi Next"
End If

WScript.Quit exitCode

Function Quote(value)
  Quote = Chr(34) & Replace(value, Chr(34), Chr(34) & Chr(34)) & Chr(34)
End Function
