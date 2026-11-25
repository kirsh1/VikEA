import { createRequire } from "node:module";

const requireForFfi = createRequire(import.meta.url);
let attached = false;

export function attachWindowsProcessJob(): void {
  if (process.platform !== "win32" || attached) return;
  if (process.arch !== "x64" && process.arch !== "arm64") {
    throw new Error("Vikea requires a 64-bit Windows runtime");
  }

  const { DataType, load, open } = requireForFfi("ffi-rs") as typeof import("ffi-rs");
  const library = "relayworkbench-kernel32";
  open({ library, path: "kernel32.dll" });
  const job = load({
    library,
    funcName: "CreateJobObjectW",
    retType: DataType.BigInt,
    paramsType: [DataType.BigInt, DataType.BigInt],
    paramsValue: [0n, 0n],
  });
  const fail = (operation: string) =>
    new Error(
      `${operation} failed (Windows error ${load({
        library,
        funcName: "GetLastError",
        retType: DataType.U32,
        paramsType: [],
        paramsValue: [],
      })})`,
    );
  if (job === 0n) throw fail("CreateJobObjectW");

  try {
    // JOBOBJECT_EXTENDED_LIMIT_INFORMATION on 64-bit Windows, with LimitFlags at offset 16.
    const limits = Buffer.alloc(144);
    limits.writeUInt32LE(0x2000, 16); // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    const configured = load({
      library,
      funcName: "SetInformationJobObject",
      retType: DataType.I32,
      paramsType: [DataType.BigInt, DataType.I32, DataType.U8Array, DataType.U32],
      paramsValue: [job, 9, limits, limits.length],
    });
    if (!configured) throw fail("SetInformationJobObject");
    const currentProcess = load({
      library,
      funcName: "GetCurrentProcess",
      retType: DataType.BigInt,
      paramsType: [],
      paramsValue: [],
    });
    const assigned = load({
      library,
      funcName: "AssignProcessToJobObject",
      retType: DataType.I32,
      paramsType: [DataType.BigInt, DataType.BigInt],
      paramsValue: [job, currentProcess],
    });
    if (!assigned) throw fail("AssignProcessToJobObject");
    // The owner is a job member. Let Windows close its only job handle at process exit.
    attached = true;
  } catch (error) {
    load({
      library,
      funcName: "CloseHandle",
      retType: DataType.I32,
      paramsType: [DataType.BigInt],
      paramsValue: [job],
    });
    throw error;
  }
}
