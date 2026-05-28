import { UIColors } from "@magic-ui/lib/types";

export interface IRootState<T> {
  settings: T;
  colors: UIColors;
}
