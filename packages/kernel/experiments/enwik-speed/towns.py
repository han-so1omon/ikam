# Bot-written town pages: one template, numbers and names vary (like enwik9's census stubs).
import os, random, sys
out, n = sys.argv[1], int(sys.argv[2])
random.seed(1)
T = """<page>
    <title>{name}, {state}</title>
    <id>{id}</id>
    <revision>
      <id>{rid}</id>
      <timestamp>2002-{m:02d}-{d:02d}T12:00:00Z</timestamp>
      <contributor><username>Rambot</username><id>4</id></contributor>
      <text xml:space="preserve">'''{name}''' is a [[city]] located in [[{county} County, {state}|{county} County]], [[{state}]]. As of the [[United States 2000 Census|2000 census]], the city had a total population of {pop}.

== Geography ==
{name} is located at {lat}&amp;deg; North, {lon}&amp;deg; West.

According to the [[United States Census Bureau]], the city has a total area of {area} km&amp;sup2; ({aream} mi&amp;sup2;). {land} km&amp;sup2; ({landm} mi&amp;sup2;) of it is land and {water} km&amp;sup2; ({waterm} mi&amp;sup2;) of it is water. The total area is {pct}% water.

== Demographics ==
As of the [[census]] of [[2000]], there are {pop} people, {hh} households, and {fam} families residing in the city. The [[population density]] is {dens}/km&amp;sup2; ({densm}/mi&amp;sup2;). There are {units} housing units at an average density of {ud}/km&amp;sup2;. The racial makeup of the city is {w}% [[White (U.S. Census)|White]], {b}% [[African American (U.S. Census)|African American]], {a}% [[Native American (U.S. Census)|Native American]], {o}% from [[Race (U.S. Census)|other races]].

There are {hh} households out of which {kids}% have children under the age of 18 living with them, {mar}% are [[Marriage|married couples]] living together, and {alone}% of all households are made up of individuals.

The median income for a household in the city is ${inc}, and the median income for a family is ${finc}. The [[per capita income]] for the city is ${pci}.

[[Category:Cities in {state}]]
[[Category:{county} County, {state}]]
</text>
    </revision>
  </page>
"""
syl = ["ab","el","or","an","ville","ton","burg","field","wood","dale","port","ham","ford","ridge","mont","lake"]
states = ["Iowa","Ohio","Texas","Kansas","Nebraska","Missouri","Indiana","Illinois","Georgia","Oregon"]
os.makedirs(out, exist_ok=True)
for i in range(n):
    r = random.random
    name = "".join(random.choice(syl) for _ in range(random.randint(2,3))).capitalize()
    v = dict(name=name, state=random.choice(states), county=random.choice(syl).capitalize()+random.choice(syl),
             id=100000+i, rid=200000+i, m=random.randint(1,12), d=random.randint(1,28),
             pop=random.randint(50,90000), lat=f"{r()*20+30:.6f}", lon=f"{r()*40+75:.6f}",
             area=f"{r()*50:.1f}", aream=f"{r()*20:.1f}", land=f"{r()*50:.1f}", landm=f"{r()*20:.1f}",
             water=f"{r()*2:.1f}", waterm=f"{r():.1f}", pct=f"{r()*5:.2f}", hh=random.randint(20,30000),
             fam=random.randint(10,20000), dens=f"{r()*900:.1f}", densm=f"{r()*2000:.1f}", units=random.randint(20,35000),
             ud=f"{r()*400:.1f}", w=f"{r()*100:.2f}", b=f"{r()*30:.2f}", a=f"{r()*5:.2f}", o=f"{r()*10:.2f}",
             kids=f"{r()*50:.1f}", mar=f"{r()*70:.1f}", alone=f"{r()*40:.1f}", inc=random.randint(15000,90000),
             finc=random.randint(15000,99000), pci=random.randint(8000,40000))
    with open(os.path.join(out, f"{i:07d}.xml"), "w") as f:
        f.write(T.format(**v))
