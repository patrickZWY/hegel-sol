// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice A compiled target used to distinguish filtering from unresolved code.
contract RegistrationHandler {
    uint256 public calls;

    function act() external {
        calls++;
    }
}

/// @notice Shared setup for registration failures with a resolvable target.
abstract contract RegistrationFixture {
    struct FuzzSelector {
        address addr;
        bytes4[] selectors;
    }

    RegistrationHandler handler;

    function setUp() public {
        handler = new RegistrationHandler();
    }

    function invariant_fixture() public pure {}

    function targetSelectors() public view virtual returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](1);
        selected[0].addr = address(handler);
        selected[0].selectors = new bytes4[](1);
        selected[0].selectors[0] = RegistrationHandler.act.selector;
    }
}

/// @notice An empty selector list on a target leaves no callable actions.
contract EmptyTargetSelectorsTest is RegistrationFixture {
    function targetSelectors() public view override returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](1);
        selected[0].addr = address(handler);
        selected[0].selectors = new bytes4[](0);
    }
}

/// @notice Contract exclusion overrides selector registration for implicit targets.
contract ExcludedSelectorTargetTest is RegistrationFixture {
    function excludeContracts() public view returns (address[] memory excluded) {
        excluded = new address[](1);
        excluded[0] = address(handler);
    }
}

/// @notice Contract exclusion also overrides explicit targets and their selectors.
contract ExcludedExplicitTargetTest is ExcludedSelectorTargetTest {
    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(handler);
    }
}

/// @notice Malformed selector results must name the registration to repair.
contract MalformedSelectorsTest {
    function targetSelectors() public pure returns (uint256) {
        return 1;
    }

    function invariant_fixture() public pure {}
}

/// @notice Malformed address results must name the registration to repair.
contract MalformedTargetsTest {
    function targetContracts() public pure returns (uint256) {
        return 1;
    }

    function invariant_fixture() public pure {}
}
