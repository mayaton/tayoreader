import { describe, expect, it } from "vitest";
import { highlight, searchTerms } from "./search";

// 強調する部分を [] で囲んだ文字列にして比べやすくする。
const marked = (text: string, terms: string[]) =>
  highlight(text, terms)
    .map((p) => (p.match ? `[${p.text}]` : p.text))
    .join("");

describe("searchTerms", () => {
  it("空白（全角空白を含む）で区切り、重複を除く", () => {
    expect(searchTerms("  台風　情報 台風 ")).toEqual(["台風", "情報"]);
    expect(searchTerms("Rust  release")).toEqual(["Rust", "release"]);
  });

  it("空白だけなら語は無い", () => {
    expect(searchTerms("")).toEqual([]);
    expect(searchTerms(" 　 ")).toEqual([]);
  });
});

describe("highlight", () => {
  it("大文字小文字を区別せずに一致部分を分ける", () => {
    expect(highlight("Rust and RUST", ["rust"])).toEqual([
      { text: "Rust", match: true },
      { text: " and ", match: false },
      { text: "RUST", match: true },
    ]);
  });

  it("複数の語をそれぞれ強調する", () => {
    expect(marked("台風情報と天気予報", ["台風", "天気"])).toBe("[台風]情報と[天気]予報");
  });

  it("重なる一致と隣り合う一致は 1 つにまとめる", () => {
    expect(marked("abcdef", ["bcd", "cde"])).toBe("a[bcde]f");
    expect(highlight("abcd", ["ab", "cd"])).toEqual([{ text: "abcd", match: true }]);
  });

  it("正規表現の特殊文字は文字どおりに扱う", () => {
    expect(marked("C++ と a.b と (x) と [y]", ["c++", ".", "(x)", "[y]"])).toBe(
      "[C++] と a[.]b と [(x)] と [[y]]",
    );
    expect(marked("a|b", ["|"])).toBe("a[|]b");
  });

  it("サロゲートペアの文字も分けられる", () => {
    expect(marked("🍣と🍺", ["🍺"])).toBe("🍣と[🍺]");
  });

  it("語が無ければ全体を 1 つ、空文字列なら何も返さない", () => {
    expect(highlight("台風", [])).toEqual([{ text: "台風", match: false }]);
    expect(highlight("", ["台風"])).toEqual([]);
  });
});
