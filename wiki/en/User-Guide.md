# Common investigations

[简体中文](../zh-CN/User-Guide.md) · [Guide home](Home.md)

## A request failed somewhere across several services

Open the related files, choose **Global** search and select the participating tabs. Search for the request ID. Expand each file’s group and click a hit to inspect the surrounding lines. To turn the matches into a single timeline, use time merge when the timestamps start in a supported format. [Search scopes](Search-and-Filters.md) · [Time merge and export](Result-Export.md)

## A service keeps writing while you investigate

Keep the service log active and turn on **View → Follow end**. To examine an earlier incident, turn following off and mark the relevant lines. The active log can still refresh while you read. [Live logs](Live-Logs.md) · [Marks and highlighting](Marks-and-Highlighting.md)

## The same incident comes back next week

Save the matching words as a predefined filter and give it a name your team will recognize. Use it in another file from the search bar’s **Filters** menu. If you share a server, publish the rule for teammates to download. [Local and cloud filters](Cloud-Filters.md)

## Someone sent a snippet instead of a file

Copy the snippet, then use **File → Open clipboard as temporary file…**. If you want to keep your edits, save the text to a file you can find again. [Clipboard and editing](Editing.md)

For the first walkthrough, use the five-line sample. For a missing match or an unexpected result, check the troubleshooting guide. [First investigation](Getting-Started.md) · [Troubleshooting](Troubleshooting.md)
