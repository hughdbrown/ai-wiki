# Workflow
The ai-wiki has several responsibilities:
- read the source file
- extract key information
-- entities
-- concepts
-- claims
-- data points
- write a summary page
- update the entity and concept pages
- flag contradictions
- update index.md
- append to log.md

# File types
1. MarkDown or text
A MarkDown or text file is processed directly in this chain.

2. Zip file
A zip file is examined to see the component parts inside the zip file. Each file within the zip file is processed separately.

3. PDF file
A PDF is examined to see its content topic. All files on technical topics are ingested. Files on these topics (among others) are not:
- court documents, anything relating to divorce
- financial documents (bank statements, investment statements, tax returns, apartment leases)
- children's report cards
If a PDF is simple, it can be ingested as is.
If a PDF is found to be an electronic book (with a title page and a table of contents), then:
- split each chapter into a separate PDF file, giving it a name that includes the book title and the chapter title/number
- ingest the split chapters
- ingest the book by summarizing it and adding links to the chapters

4. MP4, MKV
Ingest these by transcribing the audio in the file to MarkDown or text, and then ingesting the 

Actions on any file or derivative file are all logged in log.md.

5. Non-operative files
Some files are known to never be processed and operated on:
- .DMG files
These can be rejected immediately and logged by the application.

