
phone.close()
playwright.stop()

assert not FAILED, "failed scenarios:\n" + "\n".join(FAILED)
