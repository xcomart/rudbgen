package comart.rudbgen.bridge.meta;

/** H2, product name {@code "H2"}. */
final class H2Vendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.startsWith("h2");
    }
}
