import { FuncCommand } from "../../handlers/mod.ts";

import type { MonitorConfiguration as IMonitorConfiguration } from "@magic-ui/types";
import { newFromInvoke } from "../../utils/State.ts";

export class MonitorConfiguration {
  constructor(public inner: IMonitorConfiguration) {}

  static default(): Promise<MonitorConfiguration> {
    return newFromInvoke(this, FuncCommand.StateGetDefaultMonitorSettings);
  }
}
