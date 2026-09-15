# Crazyflie console

This module provides access to the Crazyflie console.

The legacy Crazyflie console and any sourced consoles are selected separately.
Running `cfcli console` without source options keeps the legacy behavior.

## Show console prints

This command shows everything printed in the Crazyflie console.

```text
cfcli console
```

If you do not want any formatting of the text then use the ```--no-format``` parameter:

```text
cfcli console --no-format
```

## Show a sourced console

Firmware using CRTP protocol 13 or newer may advertise additional console
sources, such as a console retained by a deck. List their paths with:

```text
cfcli console --list-sources
```

The list contains source paths only. Use global `--csv` to emit a `path`
header and one CSV row per source. Older firmware and Crazyflies without
sourced consoles report an empty list successfully.

Select one source by its exact, case-sensitive path:

```text
cfcli console --source deck:bcCam
```

The command first replays the source history retained for this connection and
then continues with live output. Formatting and `--no-format` behave like the
legacy console. Only one source can be selected at a time in this first
implementation; concurrent multi-source output may be added later.

If a requested source does not exist, cfcli exits with resource-not-found code
20 and reports the available paths.

## Preserve console across connections

Normally, console data is only available while connected. With the ```--preserve-console``` (```-p```) global flag, console output is saved to a file during every connection. When running multiple commands in a row the console data is accumulated:

```text
cfcli -p param set motorPowerSet.enable 1
cfcli -p log print stabilizer.roll --period 100
```

When the ```console``` command is executed, any saved console history is always printed first and then cleared, followed by the live console output:

```text
cfcli console
```

This is useful for capturing console debug output that was printed during other operations (e.g. parameter changes or log sessions).

Preservation currently applies only to the legacy Crazyflie console. A sourced
console uses its own retained history and `cfcli console --source ...` neither
prints nor clears the locally preserved legacy-console file.

## Clear preserved console history

The `--clear` flag deletes the preserved console history file and exits without connecting to a Crazyflie. Useful when you want to discard accumulated output between runs:

```text
cfcli console --clear
```

The file path is shown by `cfcli settings show`.

## Stop streaming after a fixed duration

Legacy and sourced console output are streaming commands — by default they run
until the link is broken. Combine either with the global `--timeout` flag to
stop cleanly after a fixed wall-clock duration:

```text
cfcli --timeout 3000 console
cfcli --timeout 3000 console --source deck:bcCam
```

When `--timeout` fires on a streaming command, the process exits **0** (the
timer is the intended way to stop it). For a sourced console, cfcli then makes
a clean disable attempt bounded to one additional second before disconnecting.
This is the recommended pattern when running `cfcli console` from a script or
CI step.

Ctrl-C also stops a sourced console with the same bounded disable attempt
before disconnecting, then exits with code 130. On Unix, SIGTERM follows the
same cleanup path and exits with code 143. Interruption during source enable
also attempts disable, since the device may already have accepted the request.

`--list-sources` is bounded rather than streaming. If its global timeout
expires, cfcli exits with timeout code 40.
