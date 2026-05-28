import { FuncCommand } from "../handlers/mod.ts";
import { newFromInvoke } from "../utils/State.ts";

export interface WifiNetwork {
  ssid: string;
  signal: number; // 0-100
  security: string; // secured | open | other
  connected: boolean;
}

export class WifiNetworkList extends Array<WifiNetwork> {
  static getAsync(): Promise<WifiNetworkList> {
    return newFromInvoke(this, FuncCommand.SystemGetWifiNetworks);
  }
}
