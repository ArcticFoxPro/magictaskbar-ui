import type { Profile } from "@magic-ui/types";
import { FuncCommand } from "../handlers/mod.ts";
import { List } from "../utils/List.ts";
import { newFromInvoke } from "../utils/State.ts";

export class ProfileList extends List<Profile> {
  static getAsync(): Promise<ProfileList> {
    return newFromInvoke(this, FuncCommand.StateGetProfiles);
  }
}
