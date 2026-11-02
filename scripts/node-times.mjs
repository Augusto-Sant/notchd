// A node:test reporter: one line per test, "seconds\tfile::name", for
// scripts/test-times.py. `make test` writes it to build/times/node.tsv.
import { relative } from "node:path"

export default async function* times(source) {
  for await (const event of source) {
    if (event.type !== "test:pass" && event.type !== "test:fail") continue
    const { data } = event
    if (!data.file || data.details?.type === "suite") continue
    const name = relative(process.cwd(), data.file) + "::" + data.name
    yield (data.details.duration_ms / 1000).toFixed(3) + "\t" + name + "\n"
  }
}
