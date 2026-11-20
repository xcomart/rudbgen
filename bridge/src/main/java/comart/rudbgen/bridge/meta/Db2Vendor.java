package comart.rudbgen.bridge.meta;

/** IBM Db2. */
final class Db2Vendor extends Vendor {

    @Override
    boolean names(String productName) {
        return productName.contains("db2");
    }
}
