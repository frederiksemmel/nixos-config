
phone.close()
playwright.stop()

if EXPECTED_TO_FAIL:
    assert FAILED, "every scenario passed: take this group out of expectedToFail in default.nix"
    wrong = [failure for failure in FAILED if EXPECTED_FAILURE not in failure]
    assert not wrong, f"failed for another reason than {EXPECTED_FAILURE!r}:\n" + "\n".join(wrong)
    print("failed as expected:\n" + "\n".join(FAILED))
else:
    assert not FAILED, "failed scenarios:\n" + "\n".join(FAILED)
