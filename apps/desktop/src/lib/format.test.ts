import { describe, expect, it } from "vitest";
import { relativeTime } from "./format";

describe("relativeTime", () => {
  const now = 1_790_000_000;
  it.each([
    [now, "たった今"],
    [now - 59, "たった今"],
    [now - 60, "1分前"],
    [now - 3599, "59分前"],
    [now - 3600, "1時間前"],
    [now - 86_399, "23時間前"],
    [now - 86_400, "1日前"],
    [now - 6 * 86_400, "6日前"],
    [now + 120, "たった今"],
  ])("%i → %s", (unix, expected) => {
    expect(relativeTime(unix, now)).toBe(expected);
  });

  it("7 日以上前はローカルの日付", () => {
    const unix = now - 7 * 86_400;
    const d = new Date(unix * 1000);
    expect(relativeTime(unix, now)).toBe(
      `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`,
    );
  });
});
