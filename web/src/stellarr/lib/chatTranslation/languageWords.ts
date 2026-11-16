/**
 * Function words and the commonest chat words of Polish, English and the
 * languages a message in neither is most often written in, folded the way the
 * detector reads them: lower case, no diacritics, the l with a stroke as l.
 *
 * Written for this detector, not taken from any word list. They are short on
 * purpose: words that carry grammar rather than topic, the ones every message
 * is made of whatever it is about. A word on several lists ("a", "to", "na",
 * "mi") is evidence for none of them in particular; see `detectLanguage.ts`.
 *
 * `sh` covers the South Slavic languages together (Croatian, Serbian, Bosnian,
 * Slovenian), and `sv` the Scandinavian ones: neither is told apart from its
 * neighbours here, only from Polish and English.
 */
export const WORDS = {
  pl: `
    i w z ze na do nie sie to jest a o jak co czy tak ale po od dla juz tylko jeszcze
    bardzo moze byc byl byla bylo byli sa mam masz ma mamy macie maja jestem jestes
    jestesmy ja ty on ona my wy oni one mnie mi ciebie cie ci tobie go jego jej ich im
    nam was wam nas sobie siebie moj moja moje twoj twoja twoje nasz nasza swoj swoja
    swoje ten ta te tego tej tym tych temu tu tutaj tam teraz dzis dzisiaj jutro
    wczoraj kiedy gdzie dlaczego czemu ktory ktora ktore ktos cos nic nikt nikogo
    wszystko wszyscy wszystkich wszystkim kazdy zawsze nigdy wiem wiesz wiemy chce
    chcesz chcemy mozesz moge mozemy mozna trzeba musze musisz musimy bede bedzie
    bedziesz beda by bym zeby aby bo jesli jezeli gdy jednak oraz albo lub czyli
    przez przy przed za pod nad bez miedzy u niech niz jako takze rowniez wtedy
    dopiero prawie calkiem wcale zupelnie no dobra dobrze dobry spoko dzieki dziekuje
    prosze przepraszam siema hej czesc nara okej elo tez jakis jakos gdzies kiedys
    jaki jakie ile kto czego wlasnie chyba raczej naprawde serio zaraz potem pozniej
    troche duzo bardziej nawet znowu razem sam inny nowy zrobic zrobilem zrobie
    dziala mysle widze daj dajcie znac ide idziemy czas dzien rano wieczorem chwile
    pomoc mial miala mialem bylem wiec pewnie chodzi mowi da dam fajnie kurcze kurde luz xd
  `,
  en: `
    the a an and or but of to in on at for with from by about as into over after
    before is are was were be been being am i you he she it we they me him her us
    them my your his its our their this that these those there here what which who
    when where why how not no yes do does did done have has had will would can could
    should may might must just so if then than too very really also only still again
    already now all any some every everyone anyone someone something anything nothing
    everything somebody anybody nobody more most much many other another same such
    both each else ever never always sometimes soon later ago yet anyway actually
    probably literally basically pretty quite almost enough around through without
    instead whether get got go going gonna wanna gotta know think thought want need
    like see let make made take come came say said tell told look looks work works
    try keep give find found help start stop seems wait check fix mean guess feel
    thanks thank please sorry yeah yep nope hey hi hello lol lmao omg idk imo tbh btw
    ngl brb thx u ur im dont cant wont didnt doesnt isnt thats whats ill ive today
    tomorrow yesterday tonight time way good great nice new right well back out up
    down off because though while since until maybe sure guys people thing stuff day
  `,
  cs: `
    a i v ve s se z ze k ke na do o u po od za pro pres pri pred mezi bez je jsem jsi
    jsme jste jsou neni nejsem byt byl byla bylo byli budu bude budes mam mas ma mame
    vim vis vi nevim muzu muzes muze muzeme chci chces chce chceme musim musis musi
    mel mela melo bych bys by me mne mi tebe ti te si ty vy my on ona oni mu jim nam
    vam jeho jeji jejich muj moje tvuj tvoje to ten ta toho tomu tom tim tohle tenhle
    tahle tento tato jak co kolik jaky jaka jake ktery ktera ktere nekdo neco nikdo nic
    vsechno vsichni dnes dneska zitra vcera ted tady tam moc velmi dobre spatne diky
    dekuji ahoj ano ne nebo aby kdyby tak taky take jen jenom uz jeste proc kde kdy
    kdyz protoze proste fakt jo hele nejak nekde nekdy porad furt zase znovu potom pak
    pozdeji uplne trochu hodne vic prave zrovna asi mozna opravdu docela vecer tyden
    jasne cau dik zkus
  `,
  sk: `
    a i aj v vo s so z zo k ku na do o u po od za pre cez pri pred medzi bez je som si
    sme ste su nie nebol byt bol bola bolo boli budem bude budes mam mas ma mame viem
    vies vie neviem mozem mozes moze mozeme chcem chces chce chceme musim musis musi
    mal mala malo by mna teba ti ta sa ty vy my on ona oni mu im nam vam jeho jej ich
    moj moje tvoj tvoje to ten ta toho tomu tom tym tych toto tento tato ako co kolko
    aky aka ake ktory ktora ktore niekto nieco nikto nic vsetko vsetci dnes zajtra
    vcera teraz tu tam kto velmi dost dobre zle vdaka dakujem ahoj ano nie alebo aby keby
    ked lebo pretoze tak tiez len iba uz este preco kde kedy proste fakt hej nejako
    niekde niekedy stale furt zase znova potom neskor uplne trochu vela viac prave asi
    mozno naozaj celkom vecer tyzden jasne cau skus
  `,
  sh: `
    i a u na za s sa iz od do po o kod bez je su sam si smo ste nisam nije nismo bio
    bila bilo biti bit ce cu ces hocu hoces zelim mogu mozes moze treba ja ti on ona
    mi vi oni me te ga mu joj nas vas im se moj moja tvoj tvoja ovo ono to taj ta
    koji koja koje sto sta tko ko gdje gde kada kad zasto kako koliko danas sutra jucer
    sad sada ovdje tamo vec jos samo jako vrlo puno malo dobro hvala molim bok zdravo
    da ne ili ali jer ako dok pa evo eto netko nesto nitko nista sve svi uvijek nikad
    jaz sem so ni bil bo bom lahko zelo prosim zivjo kaj kdo kje zakaj danes jutri
    vceraj zdaj tukaj ze tudi in ker ko vse vsi nekdo nekaj nic
  `,
  de: `
    der die das den dem des ein eine einen einem einer und oder aber sondern ist sind
    war waren bin bist seid sein hat habe hab hast habt haben hatte wird werden wurde
    worden kann kannst konnen konnte muss musst mussen musste soll sollte will wollte
    ich du er sie es wir ihr mich mir dich dir uns euch ihn ihm sich mein meine dein
    seine ihre unser dieser diese dieses nicht kein keine auch noch nur schon mal doch
    ja nein wie was wer wo wann warum wieso wenn dass ob weil denn dann mit von zu zum
    zur bei nach aus auf fur uber unter vor ohne gegen durch zwischen im am um bis
    heute morgen gestern jetzt gerade gleich bald spater hier dort sehr mehr viel
    nichts jemand niemand jeder alle gut danke bitte hallo tschuss genau also halt
    einfach wieder immer etwas alles gibt geht kommt macht machen weiss glaube finde
    frag kurz los
  `,
  es: `
    el la los las un una unos unas y o pero de del al en con por para sin sobre que
    como cuando donde quien es son era fue ser estar esta estoy estas estan hay he has
    ha hemos tengo tiene tienes puedo puedes puede quiero quieres voy vas va vamos hace
    hacer yo tu ella nosotros vosotros ellos me te se nos os le les lo mi su muy mas
    ya no si tambien todo todos nada algo alguien nadie mucho mucha muchos muchas poco
    otro otra vez aqui hoy manana ayer ahora todavia siempre nunca solo despues antes
    mejor bien gracias hola vale bueno pues porque entonces eso esto ese este claro
    oye mira ni sea
  `,
  fr: `
    le la les un une des du de et ou mais est sont etait sera suis es sommes etes ai
    as avons avez ont avait je tu il elle on nous vous ils elles me te se moi toi lui
    eux leur leurs mon ma mes ton ta tes son sa ses notre votre nos vos ce cet cette
    ces ca que qui quoi dans sur pour par avec sans chez au aux en y pas ne plus tres
    bien bon aussi encore deja toujours jamais rien tout tous chaque quelqu quelque
    chose personne oui non merci salut bonjour aujourd demain hier soir matin
    maintenant ici alors donc parce comme quand apres avant puis vraiment juste trop
    peu beaucoup fait faire peux peut veux veut dois doit vais va vas allez voila bah
    ben
  `,
  it: `
    il lo la i gli le un uno una di del dello della dei degli delle e o ma che chi cui
    non sono sei siamo siete sia stato essere ho hai ha abbiamo avete hanno avere era
    sto sta stai fa fare va vado posso puoi puo voglio devo deve so sa io tu lui lei
    noi voi loro mi ti ci vi si ne mio mia tuo tua suo sua per con su tra fra da dal
    dalla in nel nella nei negli sul alla al agli col anche molto tanto piu gia ancora
    sempre mai niente nessuno qualcuno qualcosa ogni volta tutto tutti solo proprio
    bene male grazie ciao buongiorno oggi domani ieri stasera adesso ora poi prima dopo
    qui dove allora quindi perche come quando questo quello cosa ecco boh dai
  `,
  nl: `
    de het een en of maar want dus omdat is zijn was waren ben bent heb hebt heeft
    hebben had kan kun kunnen zou zouden moet moeten wil zal werd geweest ik jij je hij
    zij ze wij we jullie mij me mijn jouw jou haar ons hun zich niet geen ook nog wel
    al als dat die deze dit wat wie waar wanneer waarom hoe met van voor naar door
    tegen zonder tussen sinds op in uit bij aan om te tot over onder na mee er hier
    daar nu vandaag morgen gisteren heel goed dank bedankt hoi hallo ja nee even
    gewoon weer altijd iets alles niets iemand niemand iedereen elke zo toch nou eens
    graag misschien zeker echt veel weinig terug kom komt ga gaat doe doet weet denk
    vind zie zit laat
  `,
  pt: `
    o a os as um uma de do da dos das em no na nos nas e ou mas que com por para sem
    sobre como quando onde quem eu voce ele ela eles elas me te se lhe meu minha seu
    sua nao sim muito mais ja tambem tudo nada algo alguem ninguem aqui hoje amanha
    ontem agora bem obrigado obrigada ola tchau porque entao isso isto esse este
    estou esta estao tem tenho
  `,
  ro: `
    si in la cu pe de din pentru nu da ce cine cand unde cum este sunt am ai are avem
    eu tu el ea noi voi ei mai foarte acum azi maine ieri multumesc salut bine deja
    doar care sau dar daca asa tot toti nimic ceva cineva aici acolo
  `,
  hu: `
    a az egy es is nem igen hogy van vagy volt lesz meg mar csak de ez ezt azt mi ki
    mit mikor hol miert hogyan szia koszi koszonom jo nagyon most itt ott ma holnap
    tegnap en te o ti nekem neked kell lehet tudom tudod minden valaki valami senki
    semmi sok kicsit szerintem persze akkor mert ha mint vagyok vagyunk
  `,
  sv: `
    och att det som en ett ar var har jag du han hon vi ni de inte med for pa av till
    fran om men sa nu dar vad vem nar hur varfor tack hej ja nej idag imorgon igar bra
    mycket ocksa bara redan kan ska vill eller jeg ikke og er
  `,
  tr: `
    ve bir bu su o ben sen biz siz onlar ne nasil neden nerede var yok degil evet hayir
    cok daha en icin ile gibi ama fakat ki mi mu da de simdi bugun yarin dun merhaba
    selam tesekkurler sagol tamam iyi her sey hic
  `,
} as const;

export type WordListLanguage = keyof typeof WORDS;
