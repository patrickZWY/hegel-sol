// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Forge-style handler whose selected and excluded functions have
/// different observable effects.
contract SelectorHandler {
    uint256 public calls;
    bool public forbiddenCalled;

    function allowed() external {
        calls += 1;
    }

    function forbidden() external {
        forbiddenCalled = true;
    }
}

/// @notice A selector registration that excludes a failing handler function.
contract SelectedSafeFunctionTest {
    struct FuzzSelector {
        address addr;
        bytes4[] selectors;
    }

    SelectorHandler handler;

    function setUp() public {
        handler = new SelectorHandler();
    }

    function targetSelectors() public view virtual returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](1);
        selected[0].addr = address(handler);
        selected[0].selectors = new bytes4[](1);
        selected[0].selectors[0] = SelectorHandler.allowed.selector;
    }

    function invariant_only_allowed_runs() public view virtual {
        assert(!handler.forbiddenCalled());
    }
}

/// @notice A selector registration that includes only the failing handler.
contract SelectedFailingFunctionTest is SelectedSafeFunctionTest {
    function targetSelectors() public view override returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](1);
        selected[0].addr = address(handler);
        selected[0].selectors = new bytes4[](1);
        selected[0].selectors[0] = SelectorHandler.forbidden.selector;
    }
}

/// @notice The selected safe function is actually scheduled, rather than skipped.
contract SelectedCallableFunctionTest is SelectedSafeFunctionTest {
    function invariant_only_allowed_runs() public view override {
        assert(handler.calls() == 0);
    }
}

/// @notice Repeated registrations for a target combine their selected functions.
contract MergedSelectorRegistrationTest is SelectedSafeFunctionTest {
    function targetSelectors() public view override returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](2);
        selected[0].addr = address(handler);
        selected[0].selectors = new bytes4[](1);
        selected[0].selectors[0] = SelectorHandler.allowed.selector;
        selected[1].addr = address(handler);
        selected[1].selectors = new bytes4[](1);
        selected[1].selectors[0] = SelectorHandler.forbidden.selector;
    }

    function invariant_only_allowed_runs() public view override {
        assert(handler.calls() == 0 || !handler.forbiddenCalled());
    }
}

/// @notice An empty registration list leaves deployed targets unrestricted.
contract EmptySelectorRegistrationTest is SelectedSafeFunctionTest {
    function targetSelectors() public pure override returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](0);
    }
}

/// @notice Repeated explicit target addresses represent one callable instance.
contract RepeatedAddressRegistrationTest is SelectedCallableFunctionTest {
    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](2);
        targets[0] = address(handler);
        targets[1] = address(handler);
    }
}

/// @notice Repeated selector entries for one target represent one callable instance.
contract DuplicateSelectorTargetTest is SelectedCallableFunctionTest {
    function targetSelectors() public view override returns (FuzzSelector[] memory selected) {
        selected = new FuzzSelector[](2);
        for (uint256 i; i < selected.length; i++) {
            selected[i].addr = address(handler);
            selected[i].selectors = new bytes4[](2);
            selected[i].selectors[0] = SelectorHandler.allowed.selector;
            selected[i].selectors[1] = SelectorHandler.allowed.selector;
        }
    }
}
