# Common investigations

[简体中文](../zh-CN/User-Guide.md) · [Guide home](Home.md)

## A request failed somewhere across several services

Open the related files, choose **Global** search and select the participating tabs. Search for the request ID. Expand each file’s group and click a hit to inspect the surrounding lines. To turn the matches into a single timeline, use time merge when the timestamps start in a supported format. [Try it with two service logs](Search-and-Filters.md#find-a-request-in-a-folder-on-disk) · [Check the merged order](Result-Export.md#put-two-services-on-one-timeline)

## A service keeps writing while you investigate

Keep the service log active and turn on **View → Follow end**. To examine an earlier incident, turn following off and mark the relevant lines. The active log can still refresh while you read. [Append a line and watch it arrive](Live-Logs.md#watch-one-line-arrive) · [Marks and highlighting](Marks-and-Highlighting.md)

## The same incident comes back next week

Save the matching words as a predefined filter and give it a name your team will recognize. Use it in another file from the search bar’s **Filters** menu. If you share a server, publish the rule for teammates to download. [Save a rule](Cloud-Filters.md#try-a-filter-on-your-own) · [Download a teammate’s rule](Cloud-Filters.md#download-a-teammates-rule)

## Someone sent a snippet instead of a file

Copy the snippet, then use **File → Open clipboard as temporary file…**. Saving updates the temporary file. For a lasting copy, paste the text into a new file and choose its path. [Search and keep a ticket snippet](Editing.md#search-a-snippet-copied-from-a-ticket)

For the first walkthrough, use the five-line sample. For a missing match or an unexpected result, check the troubleshooting guide. [First investigation](Getting-Started.md) · [Diagnose missing results by line number](Troubleshooting.md#search-returns-fewer-lines-than-expected)
