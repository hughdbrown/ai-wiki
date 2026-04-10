# Application
The application suite has several parts. There is a single command line application that directs to subfunctions, based on the command line interface command.

## Ingest
These command variants cause ingestion of files:
1. `app ingest <filename>`
2. `app ingest <filespec>`
3. `app ingest <directory>`
In each case, the files matching the filename or the filespec or all files in a directory are ingested.

## Terminal user interface
This command displays a terminal user interface:
`app tui`
The display shows one line per high-level ingestion activity. If an ingeted file had parts, then those parts are show as expanadble items below the main item.

For example, if a top-level file for ingestion is a ZIP file, then the items within the ZIP file each have entries for below the ZIP file for their ingestion.

Or if the top-level file is a PDF that is a book with a table of contents, then the chapters are each given ingestion entries.

Lines in the TUI show:
- item name (file name for most things; book+chapter name for a chapter in a book)
- start datetime of ingestion
- status of ingestion (queued, in-progress, complete, rejected)
- link to finished page

## Other parts
There will be other parts added as they are designed.
