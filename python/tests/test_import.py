import pycanonic
import pycanonic._core


def test_core_version():
    assert pycanonic._core.version() == pycanonic.__version__
