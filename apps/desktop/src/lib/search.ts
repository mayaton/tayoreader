// 検索語の分割と、一覧で一致部分を強調するための分割。DOM に触らない純粋関数にしてテストする。

export type Part = { text: string; match: boolean };

// 検索語を空白（全角空白を含む）で区切り、重複を除いて返す。core の検索と同じ区切り方。
export function searchTerms(query: string): string[] {
  return [...new Set(query.split(/\s+/).filter((t) => t !== ""))];
}

// text を、terms のどれかに一致する部分とそれ以外に分ける。大文字小文字は区別しない。
// 重なる一致・隣り合う一致は 1 つにまとめる。HTML としては扱わない（描画側でテキストとして出す）。
export function highlight(text: string, terms: string[]): Part[] {
  const matched = new Array<boolean>(text.length).fill(false);
  for (const term of terms) {
    for (const m of text.matchAll(new RegExp(escapeRegExp(term), "giu"))) {
      matched.fill(true, m.index, m.index + m[0].length);
    }
  }
  const parts: Part[] = [];
  let start = 0;
  for (let i = 1; i <= text.length; i++) {
    if (i === text.length || matched[i] !== matched[start]) {
      parts.push({ text: text.slice(start, i), match: matched[start] });
      start = i;
    }
  }
  return parts;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
