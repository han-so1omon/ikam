#!/usr/bin/env python3
"""E420: a small deterministic generator of TPC-DS-shaped tables.

    python3 -I tpcds_gen.py OUT_DIR [SCALE] [SEED]

Writes OUT_DIR/rows.tsv (TABLE, row key, JSON object; one line a row)
and OUT_DIR/declared.tsv (the foreign keys this generator declares:
child table, child column, parent table, parent column; a composite key
has its columns joined by commas).

Nine tables with TPC-DS's column names and key layout (a subset of each
table's columns): date_dim and time_dim at their fixed sizes, item,
customer, customer_address, store and promotion scaled from their
scale-factor-1 sizes, and store_sales and store_returns. The values are
synthetic (no dsdgen): surrogate keys are 1..N (date_dim's from the
Julian day 2415022, time_dim's seconds 0..86399), foreign keys draw from
their parent's keys with about 4% nulls in the fact tables, and a few
columns point at tables that are not generated (customer_demographics,
household_demographics, reason), with values that happen to fall inside
other tables' key ranges, as decoys.
"""

import json
import os
import random
import sys
from datetime import date, timedelta

JULIAN_1900 = 2415022  # d_date_sk of 1900-01-01


def main():
    out = sys.argv[1]
    scale = float(sys.argv[2]) if len(sys.argv) > 2 else 0.01
    seed = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    rng = random.Random(seed)

    def below(n):
        return int(rng.random() * n)

    def pick(xs):
        return xs[below(len(xs))]

    def maybe_null(v, share=0.04):
        return None if rng.random() < share else v

    def ident(i):
        # TPC-DS's 16-letter business ids, from the row number.
        s, n = "", i
        for _ in range(16):
            s = chr(ord("A") + n % 16) + s
            n //= 16
        return s

    n_item = max(200, int(18000 * scale))
    n_customer = max(1000, int(100000 * scale))
    n_address = max(500, int(50000 * scale))
    n_store = 12
    n_promo = max(30, int(300 * scale))
    n_tickets = max(2400, int(240000 * scale))
    n_cdemo, n_hdemo, n_reason = 19208, 7200, 35

    rows = []

    def put(table, key, obj):
        rows.append((table, key, json.dumps(obj, separators=(",", ":"))))

    # date_dim: 1900-01-01 to 2099-12-31, as TPC-DS.
    d0 = date(1900, 1, 1)
    n_date = 73049
    for i in range(n_date):
        d = d0 + timedelta(days=i)
        sk = JULIAN_1900 + i
        put("date_dim", f"{sk:08d}", {
            "d_date_sk": sk,
            "d_date_id": ident(sk),
            "d_date": d.isoformat(),
            "d_month_seq": (d.year - 1900) * 12 + d.month - 1,
            "d_week_seq": i // 7 + 1,
            "d_quarter_seq": (d.year - 1900) * 4 + (d.month - 1) // 3 + 1,
            "d_year": d.year,
            "d_dow": d.isoweekday() % 7,
            "d_moy": d.month,
            "d_dom": d.day,
            "d_qoy": (d.month - 1) // 3 + 1,
            "d_holiday": "Y" if (d.month, d.day) in ((1, 1), (7, 4), (12, 25)) else "N",
            "d_weekend": "Y" if d.isoweekday() >= 6 else "N",
        })
    # Sales dates as TPC-DS's: 1998-01-02 to 2003-01-02.
    sales_lo = JULIAN_1900 + (date(1998, 1, 2) - d0).days
    sales_hi = JULIAN_1900 + (date(2003, 1, 2) - d0).days

    for s in range(86400):
        put("time_dim", f"{s:05d}", {
            "t_time_sk": s,
            "t_time_id": ident(s),
            "t_time": s,
            "t_hour": s // 3600,
            "t_minute": s // 60 % 60,
            "t_second": s % 60,
            "t_am_pm": "AM" if s < 43200 else "PM",
            "t_shift": ["third", "first", "second"][s // 28800],
        })

    categories = ["Books", "Children", "Electronics", "Home", "Jewelry",
                  "Men", "Music", "Shoes", "Sports", "Women"]
    colors = ["red", "blue", "green", "white", "black", "navy", "olive"]
    for i in range(1, n_item + 1):
        cat = below(len(categories))
        rec = JULIAN_1900 + (date(1997, 10, 27) - d0).days + below(1500)
        put("item", f"{i:06d}", {
            "i_item_sk": i,
            "i_item_id": ident(i),
            "i_rec_start_date": (d0 + timedelta(days=rec - JULIAN_1900)).isoformat(),
            "i_item_desc": f"item {i} description",
            "i_current_price": round(0.5 + rng.random() * 99, 2),
            "i_wholesale_cost": round(0.2 + rng.random() * 60, 2),
            "i_brand_id": 1000000 * (cat + 1) + below(16) + 1,
            "i_brand": f"brand #{below(16) + 1}",
            "i_class_id": below(16) + 1,
            "i_category_id": cat + 1,
            "i_category": categories[cat],
            "i_manufact_id": below(1000) + 1,
            "i_size": pick(["small", "medium", "large", "N/A"]),
            "i_color": pick(colors),
            "i_manager_id": below(100) + 1,
        })

    states = ["CA", "TX", "NY", "WA", "GA", "OH", "IL", "TN", "VA", "MI"]
    for i in range(1, n_address + 1):
        put("customer_address", f"{i:06d}", {
            "ca_address_sk": i,
            "ca_address_id": ident(i),
            "ca_street_number": str(below(1000) + 1),
            "ca_street_name": pick(["Main", "Oak", "Park", "Elm", "Lake", "Hill"]),
            "ca_city": pick(["Fairview", "Midway", "Oakland", "Riverside", "Salem"]),
            "ca_state": pick(states),
            "ca_zip": f"{below(99999):05d}",
            "ca_country": "United States",
            "ca_gmt_offset": pick([-5, -6, -7, -8]),
            "ca_location_type": pick(["apartment", "condo", "single family"]),
        })

    for i in range(1, n_customer + 1):
        first_sales = sales_lo - 3650 + below(3650)
        put("customer", f"{i:07d}", {
            "c_customer_sk": i,
            "c_customer_id": ident(i),
            "c_current_cdemo_sk": maybe_null(below(n_cdemo) + 1, 0.03),
            "c_current_hdemo_sk": maybe_null(below(n_hdemo) + 1, 0.03),
            "c_current_addr_sk": below(n_address) + 1,
            "c_first_shipto_date_sk": maybe_null(first_sales + 30 + below(60), 0.03),
            "c_first_sales_date_sk": maybe_null(first_sales, 0.03),
            "c_salutation": pick(["Mr.", "Mrs.", "Ms.", "Dr.", "Sir", "Miss"]),
            "c_first_name": pick(["Alex", "Sam", "Jo", "Kim", "Lee", "Robin", "Pat"]),
            "c_last_name": pick(["Smith", "Jones", "Brown", "Lopez", "Nguyen", "Khan"]),
            "c_preferred_cust_flag": pick(["Y", "N"]),
            "c_birth_day": below(28) + 1,
            "c_birth_month": below(12) + 1,
            "c_birth_year": 1924 + below(69),
            "c_birth_country": pick(["UNITED STATES", "CANADA", "MEXICO", "PERU"]),
        })

    for i in range(1, n_store + 1):
        put("store", f"{i:03d}", {
            "s_store_sk": i,
            "s_store_id": ident(i),
            "s_rec_start_date": "1997-03-13",
            "s_closed_date_sk": None if i % 4 else sales_lo + below(sales_hi - sales_lo),
            "s_store_name": pick(["able", "ought", "bar", "ese", "anti", "cally"]),
            "s_number_employees": 200 + below(100),
            "s_floor_space": 5000000 + below(5000000),
            "s_manager": pick(["Bob Doe", "Jane Roe", "Max Poe", "Ann Coe"]),
            "s_market_id": below(10) + 1,
            "s_division_id": 1,
            "s_company_id": 1,
            "s_city": pick(["Fairview", "Midway"]),
            "s_state": "TN",
            "s_zip": f"{below(99999):05d}",
            "s_gmt_offset": -5,
            "s_tax_precentage": round(rng.random() * 0.11, 2),
        })

    for i in range(1, n_promo + 1):
        start = sales_lo + below(sales_hi - sales_lo - 60)
        put("promotion", f"{i:04d}", {
            "p_promo_sk": i,
            "p_promo_id": ident(i),
            "p_start_date_sk": maybe_null(start, 0.03),
            "p_end_date_sk": maybe_null(start + 30 + below(30), 0.03),
            "p_item_sk": maybe_null(below(n_item) + 1, 0.03),
            "p_cost": 1000,
            "p_response_target": 1,
            "p_promo_name": pick(["able", "ought", "pri", "ese", "anti", "cally"]),
            "p_channel_email": pick(["Y", "N"]),
        })

    # store_sales: tickets of 1 to 19 distinct items; store_returns: about
    # one sale line in ten, returned on or after its sale.
    sales = []
    for t in range(1, n_tickets + 1):
        day = sales_lo + below(sales_hi - sales_lo)
        tm = 28800 + below(75600 - 28800)
        cust, addr = below(n_customer) + 1, below(n_address) + 1
        store = below(n_store) + 1
        cdemo, hdemo = below(n_cdemo) + 1, below(n_hdemo) + 1
        items = set()
        for _ in range(1 + below(19)):
            items.add(below(n_item) + 1)
        for it in sorted(items):
            qty = below(100) + 1
            price = round(1 + rng.random() * 200, 2)
            sale = round(price * (0.4 + rng.random() * 0.6), 2)
            row = {
                "ss_sold_date_sk": maybe_null(day),
                "ss_sold_time_sk": maybe_null(tm),
                "ss_item_sk": it,
                "ss_customer_sk": maybe_null(cust),
                "ss_cdemo_sk": maybe_null(cdemo),
                "ss_hdemo_sk": maybe_null(hdemo),
                "ss_addr_sk": maybe_null(addr),
                "ss_store_sk": maybe_null(store),
                "ss_promo_sk": maybe_null(below(n_promo) + 1),
                "ss_ticket_number": t,
                "ss_quantity": qty,
                "ss_wholesale_cost": round(price * 0.5, 2),
                "ss_list_price": price,
                "ss_sales_price": sale,
                "ss_ext_sales_price": round(sale * qty, 2),
                "ss_net_paid": round(sale * qty * 0.97, 2),
                "ss_net_profit": round((sale - price * 0.5) * qty, 2),
            }
            put("store_sales", f"{t:08d}-{it:06d}", row)
            sales.append(row)
    for s in sales:
        if rng.random() >= 0.1:
            continue
        day = s["ss_sold_date_sk"]
        put("store_returns", f"{s['ss_ticket_number']:08d}-{s['ss_item_sk']:06d}", {
            "sr_returned_date_sk": maybe_null(None if day is None else day + below(90)),
            "sr_return_time_sk": maybe_null(28800 + below(75600 - 28800)),
            "sr_item_sk": s["ss_item_sk"],
            "sr_customer_sk": maybe_null(s["ss_customer_sk"]),
            "sr_cdemo_sk": maybe_null(below(n_cdemo) + 1),
            "sr_hdemo_sk": maybe_null(below(n_hdemo) + 1),
            "sr_addr_sk": maybe_null(s["ss_addr_sk"]),
            "sr_store_sk": maybe_null(s["ss_store_sk"]),
            "sr_reason_sk": maybe_null(below(n_reason) + 1),
            "sr_ticket_number": s["ss_ticket_number"],
            "sr_return_quantity": 1 + below(s["ss_quantity"]),
            "sr_return_amt": round(s["ss_sales_price"] * (0.5 + rng.random() * 0.5), 2),
            "sr_fee": round(0.5 + rng.random() * 99, 2),
            "sr_net_loss": round(rng.random() * 300, 2),
        })

    # The foreign keys TPC-DS declares among these nine tables.
    declared = [
        ("customer", "c_current_addr_sk", "customer_address", "ca_address_sk"),
        ("customer", "c_first_shipto_date_sk", "date_dim", "d_date_sk"),
        ("customer", "c_first_sales_date_sk", "date_dim", "d_date_sk"),
        ("store", "s_closed_date_sk", "date_dim", "d_date_sk"),
        ("promotion", "p_start_date_sk", "date_dim", "d_date_sk"),
        ("promotion", "p_end_date_sk", "date_dim", "d_date_sk"),
        ("promotion", "p_item_sk", "item", "i_item_sk"),
        ("store_sales", "ss_sold_date_sk", "date_dim", "d_date_sk"),
        ("store_sales", "ss_sold_time_sk", "time_dim", "t_time_sk"),
        ("store_sales", "ss_item_sk", "item", "i_item_sk"),
        ("store_sales", "ss_customer_sk", "customer", "c_customer_sk"),
        ("store_sales", "ss_addr_sk", "customer_address", "ca_address_sk"),
        ("store_sales", "ss_store_sk", "store", "s_store_sk"),
        ("store_sales", "ss_promo_sk", "promotion", "p_promo_sk"),
        ("store_returns", "sr_returned_date_sk", "date_dim", "d_date_sk"),
        ("store_returns", "sr_return_time_sk", "time_dim", "t_time_sk"),
        ("store_returns", "sr_item_sk", "item", "i_item_sk"),
        ("store_returns", "sr_customer_sk", "customer", "c_customer_sk"),
        ("store_returns", "sr_addr_sk", "customer_address", "ca_address_sk"),
        ("store_returns", "sr_store_sk", "store", "s_store_sk"),
        ("store_returns", "sr_item_sk,sr_ticket_number", "store_sales",
         "ss_item_sk,ss_ticket_number"),
    ]

    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "rows.tsv"), "w") as f:
        for t, k, j in rows:
            f.write(f"{t}\t{k}\t{j}\n")
    with open(os.path.join(out, "declared.tsv"), "w") as f:
        for d in declared:
            f.write("\t".join(d) + "\n")
    counts = {}
    for t, _, _ in rows:
        counts[t] = counts.get(t, 0) + 1
    print(json.dumps({"rows": len(rows), "tables": counts}))


if __name__ == "__main__":
    main()
