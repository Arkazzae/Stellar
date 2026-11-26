import { fences } from "./messageBody";

/** Match the API's limits in UTF-16 units, including Markdown delimiters. */
export const MESSAGE_MAX_LENGTH = 20_000;
export const CODE_MESSAGE_MAX_LENGTH = 100_000;

/** Code may grow past the text limit when the prose around it still fits. */
export function messageContentLimit(body: string): number {
  if (body.length <= MESSAGE_MAX_LENGTH) return MESSAGE_MAX_LENGTH;
  let proseLength = body.length;
  for (const { start, end } of fences(body)) proseLength -= end - start;
  return proseLength <= MESSAGE_MAX_LENGTH ? CODE_MESSAGE_MAX_LENGTH : MESSAGE_MAX_LENGTH;
}
