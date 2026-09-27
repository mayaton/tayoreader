// キー入力を画面の操作に対応させる。DOM に触らない純粋関数にしてテストする。

export type Action =
  | "next"
  | "prev"
  | "scrollDown"
  | "scrollUp"
  | "nextSource"
  | "prevSource"
  | "toggleRead"
  | "toggleStar"
  | "openOriginal"
  | "toggleUnreadOnly"
  | "refresh"
  | "addFeed"
  | "markAllRead"
  | "closeDialog"
  | "focusSearch"
  | "submitSearch"
  | "clearSearch";

export type KeyInput = {
  key: string;
  shiftKey: boolean;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  // IME で変換中か（確定の Enter や取り消しの Esc も含む）
  composing?: boolean;
  target: { tagName?: string; isContentEditable?: boolean; type?: string } | null;
};

const EDITABLE_TAGS = new Set(["INPUT", "TEXTAREA", "SELECT"]);

const PLAIN_KEYS: Record<string, Action> = {
  j: "next",
  ArrowDown: "next",
  k: "prev",
  ArrowUp: "prev",
  "]": "nextSource",
  "[": "prevSource",
  m: "toggleRead",
  s: "toggleStar",
  o: "openOriginal",
  Enter: "openOriginal",
  u: "toggleUnreadOnly",
  r: "refresh",
  a: "addFeed",
  "/": "focusSearch",
};

// 押されたキーに対応する操作を返す。入力欄やダイアログの操作中は Esc だけを扱う。
// 検索欄では Esc で検索の解除、Enter で検索して一覧へ戻る。
export function actionFor(input: KeyInput, context: { dialogOpen: boolean }): Action | null {
  // IME のキーは IME に任せる（変換の確定や取り消しで検索や画面の操作を動かさない）
  if (input.composing) return null;
  const inSearch = !context.dialogOpen && isSearchBox(input.target);
  if (input.key === "Escape") return inSearch ? "clearSearch" : "closeDialog";
  // Cmd+R などメニューのショートカットは邪魔しない
  if (input.metaKey || input.ctrlKey || input.altKey) return null;
  if (inSearch) return input.key === "Enter" ? "submitSearch" : null;
  if (context.dialogOpen || isEditable(input.target)) return null;
  if (input.key === " ") return input.shiftKey ? "scrollUp" : "scrollDown";
  if (input.shiftKey) return input.key === "A" ? "markAllRead" : null;
  return PLAIN_KEYS[input.key] ?? null;
}

function isSearchBox(target: KeyInput["target"]): boolean {
  return target?.tagName === "INPUT" && target.type === "search";
}

function isEditable(target: KeyInput["target"]): boolean {
  if (!target) return false;
  return target.isContentEditable === true || EDITABLE_TAGS.has(target.tagName ?? "");
}
