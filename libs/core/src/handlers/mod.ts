import type {
  FuncCommandArgument,
  FuncCommandReturn,
  FuncEventPayload,
} from "@magic-ui/types";
import {
  invoke as tauriInvoke,
  type InvokeOptions,
} from "@tauri-apps/api/core";
import {
  type EventCallback,
  listen,
  type Options as ListenerOptions,
} from "@tauri-apps/api/event";

import { FuncCommand } from "./commands.ts";
import { FuncEvent } from "./events.ts";

type $keyof<Type> = [Type] extends [never] ? keyof Type
  : Type extends Type ? keyof Type
  : never;

type UnionToIntersection<Type> = {
  [Key in $keyof<Type>]: Extract<
    Type,
    {
      [key in Key]?: unknown;
    }
  >[Key];
};

type MapNullToVoid<Obj> = {
  [K in keyof Obj]: [Obj[K]] extends [null] ? void : Obj[K];
};

type MapNullToUndefined<Obj> = {
  [K in keyof Obj]: [Obj[K]] extends [null] ? undefined : Obj[K];
};

export type AllFuncCommandArguments = MapNullToUndefined<
  UnionToIntersection<FuncCommandArgument>
>;
export type AllFuncCommandReturns = MapNullToVoid<
  UnionToIntersection<FuncCommandReturn>
>;

export type AllFuncEventPayloads = UnionToIntersection<FuncEventPayload>;

/**
 * Will call to the background process
 * @args Command to be called
 * @args Command arguments
 * @return Result of the command
 */
export function invoke<T extends FuncCommand>(
  ...args: [AllFuncCommandArguments[T]] extends [undefined] ? [
      command: T,
      args?: undefined,
      options?: InvokeOptions,
    ]
    : [
      command: T,
      args: AllFuncCommandArguments[T],
      options?: InvokeOptions,
    ]
): Promise<AllFuncCommandReturns[T]> {
  const [command, commandArgs, options] = args;
  return tauriInvoke(command, commandArgs, options);
}

export type UnSubscriber = () => void;

export function subscribe<T extends FuncEvent>(
  event: T,
  cb: EventCallback<AllFuncEventPayloads[T]>,
  options?: ListenerOptions,
): Promise<UnSubscriber> {
  return listen(event, cb, options);
}

export * from "./events.ts";
export * from "./commands.ts";
