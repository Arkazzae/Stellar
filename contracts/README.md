# Shared message-syntax contract

`message-syntax.json` is the one corpus both sides of message formatting read:

- `web/src/lib/messages/messageSyntaxContract.test.ts` checks the renderer's
  inline nodes and composer edits, and asks the transcript's own
  `mentionsUsername` about every fixed member. That is the same `mentions`
  field the API checks, so the two consumers cannot pass contradictory cases.
- `server/Stellarr.Api.Tests/MessageSyntaxContractTests.cs` checks the mention
  extractor. The test project copies the file to
  `test-contracts/message-syntax.json` in its output directory.

A grammar rule is not finished until both suites pass the same cases. When the
parser, mention extraction or a formatting button learns a rule, add the case
here instead of only to one language's tests.

## Shape

- `members` lists the usernames the web cases resolve, once each. A case that
  expects a mention must use one of these names, written upper case in
  `mentions`; the server cases expect that same spelling.
- `messages` holds authored bodies. `inline` is the renderer's expected node
  tree, `mentions` is the extractor's expected names (unique, upper case), and
  an optional `source` pins what `inlineSource` writes back (used for
  adjacent-style merging).
  Nodes are `{"text": …}`, `{"code": …}`, `{"mention": "@name"}`,
  `{"link": {"href": …, "text": …}}`, or one of `strong`, `emphasis`, `strike`
  and `spoiler` wrapping an array of child nodes.
- `composer` holds a draft, selection and marker, with the exact `TextEdit` the
  formatting button must return.

Both suites fail when the file is missing, is not valid JSON, or has a malformed
case; neither skips it.

## Running just the contract

From the repository root, with the usual .NET settings:

```sh
(cd web && npx vitest run src/lib/messages/messageSyntaxContract.test.ts)
dotnet test server/Stellarr.Api.Tests/Stellarr.Api.Tests.csproj \
  --filter FullyQualifiedName~MessageSyntaxContractTests
```
