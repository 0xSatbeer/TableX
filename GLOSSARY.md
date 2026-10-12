# QueryX

A desktop SQL client for Postgres and SQLite. These terms name the things a user manages and sees.

## Connections

**Connection**:
A saved set of settings for reaching one Postgres server or one SQLite file, plus its name, color and Production flag. A connection is not a live link; it can exist while nothing is open.
_Avoid_: datasource, server, profile

**Session**:
One live, open link to a database, opened for a connection. A Postgres connection can hold one session per database the user has expanded.
_Avoid_: live connection, handle, socket

**Connection status**:
Where a connection stands right now: Disconnected, Connecting, Connected, or Error with a reason. Only a failed connect puts a connection in Error; a failure deeper in the tree does not.
_Avoid_: state, health

**Test connection**:
A one-off check that the dialog's settings reach the database. It opens a throwaway session and never touches the connection's sessions or status.
_Avoid_: ping

## Explorer

**Introspection**:
Reading what a database contains (databases, schemas, objects, columns) from its catalogs, one tree level at a time as the user expands it.
_Avoid_: metadata load, schema scan

**Refresh**:
Re-reading one tree node's subtree from the database, so changes made elsewhere show up. It refetches the schema only, never table rows.
_Avoid_: reload, sync

**Demo mode**:
Starting QueryX with `--demo`, which shows the built-in sample workspace, sample tabs and simulated latency instead of the user's own connections.
_Avoid_: mock mode, sample mode
