import { describe, expect, it } from "vitest";

import { isolatedDeclaration, isolatedDeclarationSync } from "../index";

describe("isolated declaration", () => {
  const code = `
  /**
   * jsdoc 1
   */
  export class A {
    /**
     * jsdoc 2
     */
    foo = "bar";
  }
  // Do not keep normal comments
  export class B {}
  `;

  it("matches output", () => {
    const ret = isolatedDeclarationSync("test.ts", code, { sourcemap: true });
    expect(ret).toMatchObject({
      code:
        "/**\n"
        + "* jsdoc 1\n"
        + "*/\n"
        + "export declare class A {\n"
        + "\t/**\n"
        + "\t* jsdoc 2\n"
        + "\t*/\n"
        + "\tfoo: string;\n"
        + "}\n"
        + "export declare class B {}\n",
      map: {
        names: [],
        sources: ["test.ts"],
        sourcesContent: [code],
        version: 3,
      },
      errors: [],
    });
  });

  it("produces same result as sync", async () => {
    const syncResult = isolatedDeclarationSync("test.ts", code, { sourcemap: true });
    const asyncResult = await isolatedDeclaration("test.ts", code, { sourcemap: true });

    expect(asyncResult.code).toEqual(syncResult.code);
    expect(asyncResult.errors.length).toBe(syncResult.errors.length);
    expect(asyncResult.map).toMatchObject(syncResult.map!);
  });

  it("emits static ETS declarations only when explicitly selected", () => {
    const source = [
      "package example.declarations;",
      "export final struct Point {",
      "  x: int = 0;",
      "  move(delta: int): int { return this.x + delta; }",
      "}",
    ].join("\n");

    expect(isolatedDeclarationSync("test.ets", source).errors.length).toBeGreaterThan(0);

    const ret = isolatedDeclarationSync("test.ets", source, { lang: "ets-static" });
    expect(ret.errors).toEqual([]);
    expect(ret.code).toContain("package example.declarations;");
    expect(ret.code).toContain("export declare final struct Point");
    expect(ret.code).toContain("x: int;");
    expect(ret.code).toContain("move(delta: int): int;");
    expect(ret.code).not.toContain("return ");
  });

  it.each([isolatedDeclarationSync, isolatedDeclaration])(
    "retains types referenced by exported ambient variables",
    async (transform) => {
      const result = await transform(
        "input.ts",
        `import type { Opts } from "./opts";
interface Internal { i: 1 }
export declare const defaults: Opts;
export declare let state: Internal;`,
      );

      expect(result.errors).toEqual([]);
      expect(result.code).toBe(
        'import type { Opts } from "./opts";\n'
          + "interface Internal {\n\ti: 1;\n}\n"
          + "export declare const defaults: Opts;\n"
          + "export declare let state: Internal;\n"
          + "export {};\n",
      );
    },
  );

  it("evaluates enum template literal initializers", () => {
    const source =
      'export enum Ev { Prefix = "app", Click = `${Prefix}:click`, Multi = `line1\\nline2`, Num = `x${1}y` }';
    const result = isolatedDeclarationSync("test.ts", source);

    expect(result.errors).toEqual([]);
    expect(result.code).toBe(
      "export declare enum Ev {\n"
        + '\tPrefix = "app",\n'
        + '\tClick = "app:click",\n'
        + '\tMulti = "line1\\nline2",\n'
        + '\tNum = "x1y"\n'
        + "}\n",
    );
  });
});
