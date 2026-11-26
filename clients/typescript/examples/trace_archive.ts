/**
 * The trace domain's consumer demo (M97): one conversation over the TCP
 * door — `traceStart({ test, dir })` arms the session-side §12 archive,
 * the drive's requests land as the archive's event rows, and
 * `traceStop()` answers the finalized archive (path + the recorder
 * stats). The Attach arm stays the M18 live feed (`subscribe` /
 * `nextEvent`), unchanged here.
 *
 * ```text
 * npm run example:trace   # after `npm run build` + a running demo door
 * ```
 */

import { GwpClient } from "../src/client.ts";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const target = process.argv[2] ?? "127.0.0.1:9378";
const client = await GwpClient.connectTcp(target);
try {
  await client.handshake("trace-archive-demo/0.1");
  console.log("demo: connected to", target);

  const dir = mkdtempSync(join(tmpdir(), "gw-trace-demo-"));
  await client.traceStart({ test: "trace_demo", dir });
  console.log("demo: archive recording (test label `trace_demo`, dir", dir + ")");

  const save = await client.resolve(
    { kind: "role", role: "Button", name: { text: "Save", exact: false, ignore_case: true } },
    2000,
  );
  console.log("demo: resolved inside the recording window:", save.node.kind);

  const file = await client.traceStop();
  console.log("demo: archive finalized at", file.path);
  console.log(
    `demo: ${file.events} event rows, ${file.frames} frames, ` +
      `${file.records_dropped} records dropped, ${file.archive_bytes} bytes`,
  );
} finally {
  await client.close();
}
