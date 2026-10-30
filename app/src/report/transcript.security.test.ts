import { expect, it } from "vitest";
import { registerTranscript, reportTranscript } from "./transcript";

it("redacts untrusted registered transcripts at the report boundary", () => {
  const unregister = registerTranscript(() => [{ role: "user", text: "account_id=chatgpt-acct-123 Authorization: Basic YWJjZGVmOnBhc3N3b3Jk client_secret=abc" }]);
  try {
    const text = JSON.stringify(reportTranscript());
    for (const secret of ["chatgpt-acct-123", "YWJjZGVmOnBhc3N3b3Jk", "abc"]) expect(text).not.toContain(secret);
    expect(text).toContain("[redacted]");
  } finally {
    unregister();
  }
});
