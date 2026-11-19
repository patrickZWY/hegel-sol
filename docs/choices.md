# Choosing useful inputs

Extend [`HegelTest`](../solidity/src/Hegel.sol) to use named choices alongside the
existing `hegel.draw*` methods. Ordinary ABI-argument fuzz tests still need no helpers.
The complete transfer, order, and stateful-object examples are in
[`Choices.t.sol`](../examples/test/Choices.t.sol).

## Choose from the values your test already knows

```solidity
address from = pick("from", actors);
address to = pickOther("to", actors, from);
uint256 amount = hegel.drawUint256("amount", 0, token.balanceOf(from));
```

`actors` can be an `address[]` in memory or storage. `pickOther` excludes every copy of
`from`, so the test does not discard generated cases merely because the accounts match.
Drawing the amount within the sender's balance makes it affordable by construction.
A zero amount is included here; choose a lower bound of one only when the upper bound
is at least one.

| Helper | Result |
| --- | --- |
| `pick(name, address[])` | An address from the supplied entries. |
| `pick(name, uint256[])` | An integer from the supplied entries, such as an object ID. |
| `pickOther(name, address[], excluded)` | An address unequal to `excluded`. |
| `pickIndex(name, length)` | An index from zero through `length - 1`, for custom types. |

Names are required so a failed choice is identifiable. Zero addresses and zero integers
are ordinary candidates. Duplicates remain separate selectable entries; `pickOther`
filters only the excluded value. The helpers do not modify the supplied array. Storage
arrays are copied to memory for a call; use `pickIndex` to index a large storage array
without copying it.

## Compose a generator with ordinary Solidity

For custom structs, select an index and continue drawing fields with dependent bounds:

```solidity
Market memory market = markets[pickIndex("market", markets.length)];
uint256 affordable = budget / market.price;
uint256 maximum = affordable < market.available ? affordable : market.available;
Order memory order = Order({
    price: market.price,
    quantity: hegel.drawUint256("order.quantity", 1, maximum)
});
```

The example supplies nonzero prices, stock, and a sufficient budget. Put this code in
an internal function returning `Order` to reuse it across properties. When the domain
can have no affordable order, handle that precondition before drawing; an inverted
integer range is not a way to skip a test.

## Select objects that earlier actions created

Keep IDs in a normal Solidity collection and choose from its current contents:

```solidity
function rule_close() public {
    if (active.length == 0) return;
    uint256 id = pick("object", active);
    // Close the object and remove its ID from active.
}
```

The full example removes the chosen ID and checks that closed objects are no longer
live. The collection is restored and rebuilt with the rest of the contract state for
each generated case and replay. Use the lower-level pools when engine-tracked variable
identity or consuming a pool entry is specifically needed.

## Understand failures and replay

A choice draws a named integer, such as `from.index`, and shrinking prefers earlier
eligible entries. This is candidate order, not numeric or address order. `pickOther`
records the index in the filtered collection. Keep collection construction and ordering
deterministic for replay; changing the collection can change what an old blob selects.

`pick` and `pickOther` also append notes with the selected value:

```text
from = 0x00000000000000000000000000000000000a11ce
object = 11
```

Addresses use lowercase hexadecimal and integers use decimal. These notes do not resolve
`vm.label` aliases. `pickIndex` records only the index because it does not know your
custom value; add `hegel.note` when a field helps explain a failure. Choices use the
existing draw and note report fields and the normal reproduce command.

An empty collection, or a `pickOther` with no eligible entry, reverts with
`HegelEmptyChoice("name")` before drawing anything. This makes a missing fixture input
visible instead of silently discarding cases. Normal revert policy still applies:
Forge-style targets and handlers run with `--allow-rule-reverts` can skip a reverting
step. Guard expected empty-state preconditions explicitly, as in `rule_close` above.

These helpers are internal Solidity code using the existing runner protocol. Existing
tests and copies of `Hegel.sol` remain supported; copy the updated file to use the new
helpers. They require hegel-sol for generation, just like direct `hegel.draw*` calls.
