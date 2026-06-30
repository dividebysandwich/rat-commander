@echo off
rem rcedit -- start rat-commander directly in the editor.
rem Equivalent to `rc /edit <file>`; runs the rc.exe installed alongside this shim.
"%~dp0rc.exe" /edit %*
