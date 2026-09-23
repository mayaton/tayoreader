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
  | "closeDialog";

export type KeyInput = {
  key: string;
  shiftKey: boolean;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  target: { tagName?: string; isContentEditable?: boolean } | null;
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
};

// 押されたキーに対応する操作を返す。入力欄やダイアログの操作中は Esc だけを扱う。
export function actionFor(input: KeyInput, context: { dialogOpen: boolean }): Action | null {
  if (input.key === "Escape") return "closeDialog";
  // Cmd+R などメニューのショートカットは邪魔しない
  if (input.metaKey || input.ctrlKey || input.altKey) return null;
  if (context.dialogOpen || isEditable(input.target)) return null;
  if (input.key === " ") return input.shiftKey ? "scrollUp" : "scrollDown";
  if (input.shiftKey) return input.key === "A" ? "markAllRead" : null;
  return PLAIN_KEYS[input.key] ?? null;
}

function isEditable(target: KeyInput["target"]): boolean {
  if (!target) return false;
  return target.isContentEditable === true || EDITABLE_TAGS.has(target.tagName ?? "");
}
