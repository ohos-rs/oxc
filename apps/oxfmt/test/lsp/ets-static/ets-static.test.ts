import fs from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { TextDocument } from "vscode-languageserver-textdocument";
import { createLspConnection } from "../utils";

describe("ETS language selection", () => {
  it.each([
    { languageId: "ets-static", inMemory: false, options: {} },
    { languageId: "ets-static", inMemory: true, options: {} },
    { languageId: "ets", inMemory: false, options: { "fmt.language": "ets-static" } },
    { languageId: "ets", inMemory: true, options: { "fmt.language": "ets-static" } },
  ])("formats static ETS with $languageId (inMemory: $inMemory)", async (testCase) => {
    const root = await fs.realpath(await fs.mkdtemp(join(tmpdir(), "oxfmt-ets-")));
    try {
      await using client = createLspConnection();
      const rootUri = pathToFileURL(root).href;
      await client.initialize([{ uri: rootUri, name: "ets" }], {}, [
        { workspaceUri: rootUri, options: testCase.options },
      ]);
      const uri = testCase.inMemory ? "untitled:Box" : pathToFileURL(join(root, "Box.ets")).href;
      const source = "final class Box{value:int=1;}";
      await client.didOpen(uri, testCase.languageId, source);
      const edits = await client.format(uri);
      expect(edits?.length).toBeGreaterThan(0);
      const formatted = TextDocument.applyEdits(
        TextDocument.create(uri, testCase.languageId, 1, source),
        edits!,
      );
      expect(formatted).toBe("final class Box {\n  value: int = 1;\n}\n");
      await client.didChange(uri, formatted);
      expect((await client.format(uri)) ?? []).toEqual([]);
    } finally {
      await fs.rm(root, { recursive: true, force: true });
    }
  });
});
