/**
 * The value of a query parameter that the address carries exactly once, or
 * null.
 *
 * The links the server mails or redirects to name each parameter once. A
 * missing parameter and a repeated one both read as absent, so an edited
 * address cannot make a page choose between two tokens.
 */
export function singleQueryValue(
  params: Pick<URLSearchParams, "getAll">,
  name: string,
): string | null {
  const values = params.getAll(name);
  return values.length === 1 ? (values[0] ?? null) : null;
}
