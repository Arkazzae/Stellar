import { describe, expect, it } from "vitest";

import {
  inlineDescendants,
  inlineSource,
  inlineText,
  parseInline,
  type InlineNode,
  type ParseInlineOptions,
} from "./inline";

const text = (value: string): InlineNode => ({ kind: "text", text: value });
const code = (value: string): InlineNode => ({ kind: "code", text: value });
const link = (href: string, label = href): InlineNode => ({ kind: "link", href, text: label });
const styled =
  (kind: "strong" | "emphasis" | "strike" | "spoiler") =>
  (...children: InlineNode[]): InlineNode => ({ kind, text: inlineText(children), children });
const strong = styled("strong");
const em = styled("emphasis");
const strike = styled("strike");
const spoiler = styled("spoiler");
const ada = (typed = "@ada"): InlineNode => ({
  kind: "mention",
  text: typed,
  userId: "u-ada",
  username: "ada",
  displayName: "Ada",
  self: true,
});
const party: InlineNode = { kind: "emoji", shortcode: "party", src: "/party.webp" };

const PEOPLE: ParseInlineOptions = {
  mentions: new Map([["ada", { userId: "u-ada", username: "ada", displayName: "Ada", self: true }]]),
  emoji: new Map([["party", "/party.webp"]]),
};

type Case = [input: string, expected: InlineNode[]];

const CASES: Record<string, Case[]> = {
  emphasis: [
    ["**bold**", [strong(text("bold"))]],
    ["*italic* and _italic_", [em(text("italic")), text(" and "), em(text("italic"))]],
    // Two underscores pair the way two asterisks do.
    ["__bold__", [strong(text("bold"))]],
    ["**a *b* c**", [strong(text("a "), em(text("b")), text(" c"))]],
    ["*a **b** c*", [em(text("a "), strong(text("b")), text(" c"))]],
    ["***x***", [em(strong(text("x")))]],
    ["**ważne** i *pochylone*", [strong(text("ważne")), text(" i "), em(text("pochylone"))]],
    // `*` may pair inside a word, `_` may not.
    ["w*ord* and w**ord**s", [text("w"), em(text("ord")), text(" and w"), strong(text("ord")), text("s")]],
    ["snake_case_name and foo__bar__baz", [text("snake_case_name and foo__bar__baz")]],
    ["пристаням_стремятся_", [text("пристаням_стремятся_")]],
    ["*a*b and _a_b", [em(text("a")), text("b and _a_b")]],
    // Whitespace next to a marker keeps it from opening or closing.
    ["*foo bar *", [text("*foo bar *")]],
    ["a * foo bar*", [text("a * foo bar*")]],
    ["** a**", [text("** a**")]],
    // Punctuation: an opener before punctuation needs space or punctuation before it.
    ["*(a)* and a*(b)*", [em(text("(a)")), text(" and a*(b)*")]],
    ["*🎉*", [em(text("🎉"))]],
    ["*\"quote\"*", [em(text('"quote"'))]],
    // Unequal runs leave the surplus as text.
    ["**foo*", [text("*"), em(text("foo"))]],
    ["*foo**", [em(text("foo")), text("*")]],
    ["***foo**", [text("*"), strong(text("foo"))]],
    ["**foo***", [strong(text("foo")), text("*")]],
    // The rule of three.
    ["*foo**bar*", [em(text("foo**bar"))]],
    ["foo***bar***baz", [text("foo"), em(strong(text("bar"))), text("baz")]],
    ["foo******bar*********baz", [text("foo"), strong(strong(strong(text("bar")))), text("***baz")]],
    ["*(*foo*)*", [em(text("("), em(text("foo")), text(")"))]],
    // The first closer pairs with the nearest opener of its kind; what it
    // jumps over stays text.
    ["*a _b* c_", [em(text("a _b")), text(" c_")]],
    ["**a *b** c*", [em(em(text("a "), em(text("b"))), text(" c"))]],
    ["__a*b__ c*", [strong(text("a*b")), text(" c*")]],
    ["**niedomknięte", [text("**niedomknięte")]],
    ["5*6*78", [text("5"), em(text("6")), text("78")]],
    ["**a `*` b**", [strong(text("a "), code("*"), text(" b"))]],
    ["**a\nb**", [strong(text("a\nb"))]],
  ],
  strike: [
    ["~~stare~~", [strike(text("stare"))]],
    ["a~~b~~c", [text("a"), strike(text("b")), text("c")]],
    // Exactly two tildes.
    ["~one~ and ~~~three~~~", [text("~one~ and ~~~three~~~")]],
    ["~~a~", [text("~~a~")]],
    ["~a~~", [text("~a~~")]],
    ["~~ a~~", [text("~~ a~~")]],
    ["~~**a**~~ **~~b~~**", [strike(strong(text("a"))), text(" "), strong(strike(text("b")))]],
    ["~~😀~~", [strike(text("😀"))]],
    ["~/projects and ~user", [text("~/projects and ~user")]],
  ],
  code: [
    ["`code`", [code("code")]],
    ["x `*a*` y", [text("x "), code("*a*"), text(" y")]],
    // A run closes only on a run of the same length.
    ["``a ` b``", [code("a ` b")]],
    ["`a``", [text("`a``")]],
    ["``a`", [text("``a`")]],
    ["`a`` b`", [code("a`` b")]],
    // One space comes off each end when both ends have one.
    ["` a `", [code("a")]],
    ["`` `a` ``", [code("`a`")]],
    ["` a`", [code(" a")]],
    ["`  `", [code("  ")]],
    ["`  a  `", [code(" a ")]],
    // Everything inside is literal, line breaks and backslashes included.
    ["`a\nb`", [code("a\nb")]],
    ["`a\\`b`", [code("a\\"), text("b`")]],
    ["`<b>&amp;\"`", [code('<b>&amp;"')]],
    ["`[a](http://x)` `||s||` `\\*`", [code("[a](http://x)"), text(" "), code("||s||"), text(" "), code("\\*")]],
  ],
  spoilers: [
    ["||spoiler||", [spoiler(text("spoiler"))]],
    [
      "||**zakończenie** filmu|| a || nie",
      [spoiler(strong(text("zakończenie")), text(" filmu")), text(" a || nie")],
    ],
    ["a || b", [text("a || b")]],
    // Spaces inside are content; nothing but spaces is no spoiler.
    ["|| a || and || b||", [spoiler(text(" a ")), text(" and "), spoiler(text(" b"))]],
    ["|| ||", [text("|| ||")]],
    ["||||", [text("||||")]],
    ["|| ||a||", [text("|| "), spoiler(text("a"))]],
    ["x||y||z", [text("x"), spoiler(text("y")), text("z")]],
    ["||a|| ||b||", [spoiler(text("a")), text(" "), spoiler(text("b"))]],
    ["||a||||b||", [spoiler(text("a")), spoiler(text("b"))]],
    ["|||a|||", [text("|"), spoiler(text("a")), text("|")]],
    ["a | b | c", [text("a | b | c")]],
    ["||*a||*", [spoiler(text("*a")), text("*")]],
    ["*||a*||", [em(text("||a")), text("||")]],
    ["||`a||`||", [spoiler(code("a||"))]],
    ["||~~*a*~~||", [spoiler(strike(em(text("a"))))]],
    ["||a\nb||", [spoiler(text("a\nb"))]],
  ],
  escapes: [
    ["\\*a\\* \\_b\\_ \\~\\~c\\~\\~ \\|\\|d\\|\\|", [text("*a* _b_ ~~c~~ ||d||")]],
    ["\\`a` and \\[a](http://x)", [text("`a` and [a]("), link("http://x"), text(")")]],
    ["\\\\*a*", [text("\\"), em(text("a"))]],
    ["\\**a*", [text("*"), em(text("a"))]],
    ["*a\\*b*", [em(text("a*b"))]],
    // Only ASCII punctuation can be escaped; any other backslash is a backslash.
    ["\\a \\ą \\", [text("\\a \\ą \\")]],
    ["C:\\Users\\ada\\*.txt", [text("C:\\Users\\ada*.txt")]],
    ["raz\\\ndwa", [text("raz\\\ndwa")]],
    ["\\&amp; \\<b>", [text("&amp; <b>")]],
  ],
  links: [
    ["[strona](https://example.com)", [link("https://example.com", "strona")]],
    ["[a](HTTP://X.TEST)", [link("HTTP://X.TEST", "a")]],
    ["[a](http://x/a_(b)_c)", [link("http://x/a_(b)_c", "a")]],
    ["[x](https://pl.wikipedia.org/wiki/Kraków_(miasto))", [link("https://pl.wikipedia.org/wiki/Kraków_(miasto)", "x")]],
    ["[a](http://x)b)", [link("http://x", "a"), text("b)")]],
    // The label is plain text.
    ["[*a* **b** `c`](http://x)", [link("http://x", "a b c")]],
    ["[a [b] c](http://x) and [[a]](http://x)", [link("http://x", "a [b] c"), text(" and "), link("http://x", "[a]")]],
    ["[a\\]b](http://x)", [link("http://x", "a]b")]],
    ["[`a]`](http://x)", [link("http://x", "a]")]],
    ["[http://a](http://b)", [link("http://b", "http://a")]],
    ["[](http://x) and [ ](http://y)", [link("http://x"), text(" and "), link("http://y")]],
    // Emphasis cannot reach into or out of a label.
    ["*a [b*](http://x)", [text("*a "), link("http://x", "b*")]],
    ["*a [b](http://x) c*", [em(text("a "), link("http://x", "b"), text(" c"))]],
    ["**[dokumentacja](https://docs.test)**", [strong(link("https://docs.test", "dokumentacja"))]],
    // No links in links: the inner one wins.
    [
      "[a [b](http://x) c](http://y)",
      [text("[a "), link("http://x", "b"), text(" c]("), link("http://y"), text(")")],
    ],
    // Any other scheme or shape is literal.
    ["[a](javascript:alert(1))", [text("[a](javascript:alert(1))")]],
    ["[a](data:text/html,x) [b](ftp://x) [c](#) [d]()", [text("[a](data:text/html,x) [b](ftp://x) [c](#) [d]()")]],
    ["[a](//x.test) [b](x.test)", [text("[a](//x.test) [b](x.test)")]],
    ["[a](http://)", [text("[a]("), link("http://"), text(")")]],
    ["[a](http://x y)", [text("[a]("), link("http://x"), text(" y)")]],
    ['[a](http://x "title")', [text("[a]("), link("http://x"), text(' "title")')]],
    ["[a](<http://x>)", [text("[a](<"), link("http://x"), text(">)")]],
    ["[a](http://x(y)", [text("[a]("), link("http://x(y)")]],
    ["[a] (http://x)", [text("[a] ("), link("http://x"), text(")")]],
    ["[a][b] and [a] and [a][]", [text("[a][b] and [a] and [a][]")]],
    ["[a]: http://x", [text("[a]: "), link("http://x")]],
  ],
  literal: [
    ['<img src=x onerror="alert(1)">', [text('<img src=x onerror="alert(1)">')]],
    ["nowy <code>VoiceClient</code> i <b>Jo</b>", [text("nowy <code>VoiceClient</code> i <b>Jo</b>")]],
    ["<code>**a**</code>", [text("<code>"), strong(text("a")), text("</code>")]],
    ["a < b && c > \"d\"", [text('a < b && c > "d"')]],
    ["&amp; &lt; &#65; &#x42;", [text("&amp; &lt; &#65; &#x42;")]],
    ["![obraz](https://tracker.test/a.png)", [text("![obraz]("), link("https://tracker.test/a.png"), text(")")]],
    ["\\![a](http://x)", [text("!"), link("http://x", "a")]],
    ["<http://x.test>", [text("<"), link("http://x.test"), text(">")]],
    ["<a@b.com> a@b.com mailto:a@b.com www.example.com", [text("<a@b.com> a@b.com mailto:a@b.com www.example.com")]],
    ["ftp://x.test HTTP://X.TEST", [text("ftp://x.test HTTP://X.TEST")]],
  ],
  urls: [
    ["zobacz https://example.com/a#b tutaj", [text("zobacz "), link("https://example.com/a#b"), text(" tutaj")]],
    ["patrz https://example.com/a.", [text("patrz "), link("https://example.com/a"), text(".")]],
    ["(https://example.com/a)", [text("("), link("https://example.com/a"), text(")")]],
    ["https://example.com/a_(b)", [link("https://example.com/a_(b)")]],
    // The markers inside an address are part of it.
    ["https://x.test/pkg/__init__.py", [link("https://x.test/pkg/__init__.py")]],
    ["https://x.test/_ada_ and https://x.test/a_b_c", [link("https://x.test/_ada_"), text(" and "), link("https://x.test/a_b_c")]],
    ["http://x.test/*a* and http://x.test/~user", [link("http://x.test/*a*"), text(" and "), link("http://x.test/~user")]],
    ["https://x.test/?q=a||b", [link("https://x.test/?q=a||b")]],
    // The markers around it still format it.
    ["**https://example.org/a**", [strong(link("https://example.org/a"))]],
    ["||https://example.org/a||", [spoiler(link("https://example.org/a"))]],
    ["*https://x.test*, _https://x.test/a_", [em(link("https://x.test")), text(", "), em(link("https://x.test/a"))]],
    ["~~https://x.test/old~~.", [strike(link("https://x.test/old")), text(".")]],
    ["`https://x.test`", [code("https://x.test")]],
    ["[see https://x.test](https://y.test)", [link("https://y.test", "see https://x.test")]],
    // Inside an address a backslash is part of it; in the scheme it breaks it.
    ["https://x.test/a\\_b", [link("https://x.test/a\\_b")]],
    ["http\\://x.test", [text("http://x.test")]],
  ],
  mentionsAndEmoji: [
    [
      "Hej @Ada, `@ada` i https://x.test/@ada oraz @nikt",
      [
        text("Hej "),
        ada("@Ada"),
        text(", "),
        code("@ada"),
        text(" i "),
        link("https://x.test/@ada"),
        text(" oraz @nikt"),
      ],
    ],
    ["write to bob@ada now, @ada_ and @adam", [text("write to bob@ada now, @ada_ and @adam")]],
    ["hej :party: no, o :nieistnieje: i 18:30", [text("hej "), party, text(" no, o :nieistnieje: i 18:30")]],
    [
      "**@ada *ważne* `kod` :party:**",
      [strong(ada(), text(" "), em(text("ważne")), text(" "), code("kod"), text(" "), party)],
    ],
    ["~~@ada~~ i ||:party:||", [strike(ada()), text(" i "), spoiler(party)]],
    ["*@ada*, _@ada_ and **@ada**!", [em(ada()), text(", "), em(ada()), text(" and "), strong(ada()), text("!")]],
    ["[hej @ada :party:](https://x.test)", [link("https://x.test", "hej @ada :party:")]],
    // An escaped character is never part of a mention, an emoji or a URL.
    ["\\@ada \\:party: @ada\\_", [text("@ada :party: "), ada(), text("_")]],
    ["`:party:` and :party:`x`", [code(":party:"), text(" and "), party, code("x")]],
  ],
  newlines: [
    ["raz\ndwa", [text("raz\ndwa")]],
    ["raz  \ndwa", [text("raz  \ndwa")]],
    ["*a\nb* and ||c\nd||", [em(text("a\nb")), text(" and "), spoiler(text("c\nd"))]],
  ],
};

describe("parseInline", () => {
  for (const [group, cases] of Object.entries(CASES)) {
    describe(group, () => {
      it.each(cases)("%j", (input, expected) => {
        expect(parseInline(input, PEOPLE)).toEqual(expected);
      });
    });
  }

  it("leaves mentions and emoji alone without members or emoji to resolve them", () => {
    expect(parseInline("@ada :party:")).toEqual([text("@ada :party:")]);
  });

  it("finds mentions nested in formatting", () => {
    const nodes = parseInline("||**@ada** :party:||", PEOPLE);

    expect(inlineDescendants(nodes).map((node) => node.kind)).toEqual([
      "spoiler",
      "strong",
      "mention",
      "text",
      "emoji",
    ]);
  });
});

describe("parseInline nesting", () => {
  /** `**x ` opened `levels` times around `a`, each closed by ` y**`. */
  const nested = (levels: number) => `${"**x ".repeat(levels)}a${" y**".repeat(levels)}`;

  /** 32 levels of strong emphasis, the innermost holding `innermost` as plain text. */
  function thirtyTwoLevels(innermost: string): InlineNode[] {
    let node = strong(text(innermost));
    for (let level = 1; level < 32; level++) node = strong(text("x "), node, text(" y"));
    return [node];
  }

  it("formats 32 levels", () => {
    expect(parseInline(nested(32))).toEqual(thirtyTwoLevels("x a y"));
  });

  it("shows anything deeper as it was typed, without reading mentions in it", () => {
    expect(parseInline(nested(33))).toEqual(thirtyTwoLevels("x **x a y** y"));
    expect(parseInline(nested(40))).toEqual(thirtyTwoLevels(`x ${nested(8)} y`));
    const deep = `${"**x ".repeat(33)}@ada${" y**".repeat(33)}`;
    expect(inlineDescendants(parseInline(deep, PEOPLE)).some((node) => node.kind === "mention")).toBe(false);
  });
});

describe("inlineSource", () => {
  // What the translator is given has to come back as the same message.
  const ROUND_TRIPS = [
    "**bold**, *italic*, ~~gone~~ and ||hidden||",
    "**a *b* c** and ~~**a**~~",
    "\\*not italic\\* and snake_case and 5 * 3",
    "`code` and ``a ` b`` and `` `x` `` and ` a `",
    "[docs](https://docs.test/a_(b)) and https://x.test/a.",
    "[**bold** label](http://x) and [a\\]b](http://x)",
    "[see https://x.test and https://x.test/a*](https://y.test)",
    "**\\*star\\* a\\|\\|b \\@ada \\:party:**",
    "@ada :party: ||@ada|| `@ada`",
    "<b>Jo</b> &amp; \\\\ \\[x\\]",
    "http\\://x.test and https\\://x.test/a.",
    "https://x.test/a* and https://x.test/b_ and https://x.test/c.",
  ];

  it.each(ROUND_TRIPS)("%j", (body) => {
    const nodes = parseInline(body, PEOPLE);
    expect(parseInline(inlineSource(nodes), PEOPLE)).toEqual(nodes);
  });

  it("writes code that holds backticks inside a longer fence", () => {
    expect(inlineSource([code("a``b")])).toBe("```a``b```");
    expect(inlineSource([code("`a")])).toBe("`` `a ``");
  });

  it("keeps a literal address literal instead of turning it into a link", () => {
    const nodes = parseInline("http\\://x.test");
    expect(nodes).toEqual([text("http://x.test")]);
    expect(parseInline(inlineSource(nodes))).toEqual(nodes);
    expect(inlineSource(nodes)).toBe("http\\://x.test");
  });

  it("writes a bare link whose address ends in a marker so the address survives", () => {
    for (const address of ["https://x.test/a*", "https://x.test/b_", "https://x.test/c."]) {
      const nodes = parseInline(address);
      expect(nodes[0]).toMatchObject({ kind: "link" });
      expect(parseInline(inlineSource(nodes))).toEqual(nodes);
    }
  });

  it("merges neighbouring runs of one style instead of exposing their markers", () => {
    expect(inlineSource(parseInline("**a**__b__"))).toBe("**ab**");
    expect(inlineSource(parseInline("*a*_b_"))).toBe("*ab*");
    expect(inlineSource(parseInline("_a_*b*"))).toBe("*ab*");
  });

  it("renders adjacent same-style runs the same after the round trip", () => {
    // Two boxes of one style look the same as one; the characters inside keep
    // that style either way. What must not happen is a pair of visible stars.
    const signature = (nodes: readonly InlineNode[], styles: readonly string[] = []): string =>
      nodes
        .map((node) => {
          if (node.kind === "strong" || node.kind === "emphasis" ||
              node.kind === "strike" || node.kind === "spoiler") {
            return signature(node.children ?? [], [...styles, node.kind]);
          }
          if (node.kind !== "text" && node.kind !== "code") return "";
          return [...node.text]
            .map((char) => `${styles.join("+")}:${char}`)
            .join(",");
        })
        .join(",");

    for (const source of ["**a**__b__", "*a*_b_", "_a_*b*", "~~a~~~~b~~", "||a||||b||"]) {
      const nodes = parseInline(source);
      const markup = inlineSource(nodes);
      expect(markup).not.toContain("****");
      expect(signature(parseInline(markup))).toBe(signature(nodes));
    }
  });

  it("drops the edit mark nobody typed", () => {
    expect(inlineSource([text("a"), { kind: "edited", at: "2026-01-01T00:00:00Z" }])).toBe("a");
  });
});

describe("parseInline on hostile input", () => {
  // Each body is twice the 100 kB a stranger could be expected to try, ten
  // times what the server accepts. A linear pass takes tens of milliseconds;
  // the bound is far above that and far below what a quadratic step would
  // take at this size.
  const SIZE = 200_000;
  const BOUND_MS = 2_500;
  const fill = (unit: string) => unit.repeat(Math.ceil(SIZE / unit.length));

  const HOSTILE: [name: string, body: string][] = [
    ["asterisks", fill("*")],
    ["underscores", fill("_")],
    ["tildes", fill("~")],
    ["double tildes", fill("~~a ")],
    ["backticks", fill("`")],
    ["backtick runs of every length", Array.from({ length: 630 }, (_, n) => "`".repeat(n + 1)).join("a")],
    ["opening brackets", fill("[")],
    ["closing brackets", fill("]")],
    ["pipes", fill("||")],
    ["spaced pipes", fill("|| ")],
    ["backslashes", fill("\\")],
    ["escapes", fill("\\*")],
    ["underscores inside words", fill("_a_")],
    ["openers that never close", fill("*a ")],
    ["double openers", fill("**a")],
    ["closed spans", fill("*a*b")],
    ["mixed runs for the rule of three", fill("*a**b***c ")],
    ["underscore runs between asterisks", `*${fill("_a ")}*`],
    ["unclosed code", fill("``a")],
    ["unclosed links", fill("[a](b")],
    ["unclosed http links", fill("[a](http://a(")],
    ["links sharing one far parenthesis", `${fill("[a](http://")})`],
    ["brackets before a far target", `${fill("[a")}](http://x)`],
    ["legacy tags", fill("<b>a")],
    ["unclosed legacy code tags", fill("<code>")],
    ["spoiler openers", fill("||a")],
    ["at signs", fill("a@")],
    ["one long address", ` ${"a".repeat(SIZE)}@`],
    ["URLs with open parentheses", fill("http://a(")],
    ["many bare URLs without markers", "http://a ".repeat(125_000)],
    ["a URL with trailing dots", `http://a${fill(".")}`],
    ["a URL with closing parentheses", `http://a${fill(")")}`],
    ["a URL with trailing markers", `**http://a${fill("*_")}`],
    ["URLs and markers", fill("http://a/*_ ")],
    ["shortcode colons", fill(":a")],
    ["nested emphasis", `${"*(".repeat(SIZE / 4)}a${")*".repeat(SIZE / 4)}`],
    ["nested spoilers", `${"||a ".repeat(SIZE / 8)}${"b||".repeat(SIZE / 8)}`],
    ["nested links", `${"[".repeat(SIZE / 12)}x${"](http://a)".repeat(SIZE / 12)}`],
    ["one deep run", `${"*".repeat(SIZE / 2)}a${"*".repeat(SIZE / 2)}`],
    ["one deep spoiler run", `${"|".repeat(SIZE / 2)}a${"|".repeat(SIZE / 2)}`],
    ["mixed markers", fill("**a _b ~~c ||d `e [f]( <g h@i.j http://k( ")],
  ];

  it.each(HOSTILE)("stays fast on %s", (_name, body) => {
    const started = performance.now();
    const nodes = parseInline(body, PEOPLE);
    const elapsed = performance.now() - started;

    expect(elapsed).toBeLessThan(BOUND_MS);
    expect(inlineText(inlineDescendants(nodes).filter((node) => !("children" in node))).length)
      .toBeGreaterThan(0);
  });
});
