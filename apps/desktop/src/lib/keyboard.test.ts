import { describe, expect, it } from "vitest";
import { actionFor, type KeyInput } from "./keyboard";

const body = { tagName: "BODY" };
const key = (k: string, extra: Partial<KeyInput> = {}): KeyInput => ({
  key: k,
  shiftKey: false,
  target: body,
  ...extra,
});
const closed = { dialogOpen: false };

describe("actionFor", () => {
  it.each([
    ["j", "next"],
    ["ArrowDown", "next"],
    ["k", "prev"],
    ["ArrowUp", "prev"],
    ["]", "nextSource"],
    ["[", "prevSource"],
    ["m", "toggleRead"],
    ["s", "toggleStar"],
    ["o", "openOriginal"],
    ["Enter", "openOriginal"],
    ["u", "toggleUnreadOnly"],
    ["r", "refresh"],
    ["a", "addFeed"],
    [" ", "scrollDown"],
    ["Escape", "closeDialog"],
  ])("%s → %s", (k, action) => {
    expect(actionFor(key(k), closed)).toBe(action);
  });

  it("Shift の組み合わせ", () => {
    expect(actionFor(key(" ", { shiftKey: true }), closed)).toBe("scrollUp");
    expect(actionFor(key("A", { shiftKey: true }), closed)).toBe("markAllRead");
    // Caps Lock だけの大文字や Shift+J は何もしない
    expect(actionFor(key("A"), closed)).toBeNull();
    expect(actionFor(key("J", { shiftKey: true }), closed)).toBeNull();
  });

  it("割り当ての無いキーと修飾キー付きは無視する", () => {
    expect(actionFor(key("x"), closed)).toBeNull();
    expect(actionFor(key("r", { metaKey: true }), closed)).toBeNull();
    expect(actionFor(key("a", { ctrlKey: true }), closed)).toBeNull();
    expect(actionFor(key("j", { altKey: true }), closed)).toBeNull();
  });

  it("入力欄にフォーカスがあるときは Esc 以外を無視する", () => {
    for (const target of [
      { tagName: "INPUT" },
      { tagName: "TEXTAREA" },
      { tagName: "SELECT" },
      { tagName: "DIV", isContentEditable: true },
    ]) {
      expect(actionFor(key("j", { target }), closed)).toBeNull();
      expect(actionFor(key(" ", { target }), closed)).toBeNull();
      expect(actionFor(key("Escape", { target }), closed)).toBe("closeDialog");
    }
  });

  it("ダイアログ表示中は Esc 以外を無視する", () => {
    const open = { dialogOpen: true };
    expect(actionFor(key("j"), open)).toBeNull();
    expect(actionFor(key("A", { shiftKey: true }), open)).toBeNull();
    expect(actionFor(key("Escape"), open)).toBe("closeDialog");
  });

  it("ボタンにフォーカスがあってもショートカットは効く", () => {
    expect(actionFor(key("j", { target: { tagName: "BUTTON" } }), closed)).toBe("next");
    expect(actionFor(key("j", { target: null }), closed)).toBe("next");
  });
});
