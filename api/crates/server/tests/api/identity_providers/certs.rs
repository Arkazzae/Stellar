//! A certificate authority and a certificate for `localhost` and `127.0.0.1`,
//! made once with openssl for these tests alone: the fake provider answers
//! with the certificate, and the server under test is told to trust the
//! authority through `SSL_CERT_FILE`. The authority's own key was thrown
//! away, so nothing else can be signed with it, and it is trusted nowhere
//! outside a test's own server.

pub const CA: &str = "-----BEGIN CERTIFICATE-----
MIIBfDCCASKgAwIBAgIUSOB1ErjSHKXwXE4SUaHbh0Ssh9MwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQU3RlbGxhcnIgdGVzdCBDQTAgFw0yNjExMjQxMDI3NThaGA8y
MTI2MTAzMTEwMjc1OFowGzEZMBcGA1UEAwwQU3RlbGxhcnIgdGVzdCBDQTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABIPhX5pVCH2lul5oT13OO9YgyUt6K2X90zZ/
sNMGmCs/pfHeirLZ22j/cwqmF+l3bTH+9cky+OUbKiKR5Xz31K+jQjBAMB0GA1Ud
DgQWBBTD72f3432L8mGmJP+Kigr5CNQhEzAPBgNVHRMBAf8EBTADAQH/MA4GA1Ud
DwEB/wQEAwIBBjAKBggqhkjOPQQDAgNIADBFAiAekmoDX5nnbofrCnpLmvkDCK2+
GQIfjSsc5Q8QrzkWHwIhALhhixE9LD6rzA2p2MBf+VSKGFNm8M6jqFH80S61j83T
-----END CERTIFICATE-----
";

pub const CERTIFICATE: &str = "-----BEGIN CERTIFICATE-----
MIIBlDCCATqgAwIBAgIUKxVw6kbXJWDKTF3TuuvV9jJOsIgwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQU3RlbGxhcnIgdGVzdCBDQTAgFw0yNjExMjQxMDI3NThaGA8y
MTI2MTAzMTEwMjc1OFowGDEWMBQGA1UEAwwNZmFrZS1wcm92aWRlcjBZMBMGByqG
SM49AgEGCCqGSM49AwEHA0IABEvWPd/Kc+vLzN3bjDaW7C+UT8irbCA1cSqz60a3
WUuyUk4DfZHQ8gRkl1DpaiTZphj9bbl3Gngbq4/84DCRbbOjXTBbMCwGA1UdEQQl
MCOCCWxvY2FsaG9zdIcEfwAAAYcQAAAAAAAAAAAAAAAAAAAAATAJBgNVHRMEAjAA
MAsGA1UdDwQEAwIHgDATBgNVHSUEDDAKBggrBgEFBQcDATAKBggqhkjOPQQDAgNI
ADBFAiAZPFNgNcGde63RmMqRQ+Wj7p2OA0/aksbOjNpZLp0SdgIhAO4kcFK2zr+K
pcYXIj0Dxkitc1tnD3C54BWU0aCfzDLw
-----END CERTIFICATE-----
";

pub const KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcIit09K3wN+4Sv22
s/4jd58CQYguj36xO4IG2Ems2qyhRANCAARL1j3fynPry8zd24w2luwvlE/Iq2wg
NXEqs+tGt1lLslJOA32R0PIEZJdQ6Wok2aYY/W25dxp4G6uP/OAwkW2z
-----END PRIVATE KEY-----
";
