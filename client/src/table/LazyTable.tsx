import { useEffect, useState } from "preact/hooks";
import type { FunctionComponent } from "preact";
import type { TableView } from "../protocol/TableView";

type TableComp = FunctionComponent<{ table: TableView }>;

let cached: TableComp | null = null;
let loading: Promise<TableComp> | null = null;

/** Loads the table renderer on first use so the join screen stays small. */
export function loadTable(): Promise<TableComp> {
  loading ??= import("./TableView").then((m) => (cached = m.TableView));
  return loading;
}

export function LazyTable({ table }: { table: TableView }) {
  const [Comp, setComp] = useState<TableComp | null>(() => cached);
  useEffect(() => {
    if (!Comp) {
      let live = true;
      loadTable().then((c) => live && setComp(() => c));
      return () => {
        live = false;
      };
    }
  }, []);
  return Comp ? <Comp table={table} /> : <p class="t-muted">Loading the table…</p>;
}
