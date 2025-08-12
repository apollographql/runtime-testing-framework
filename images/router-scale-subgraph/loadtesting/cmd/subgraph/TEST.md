curl -i -H 'Content-Type: application/json' -X POST\
-d '{"query": "subscription blah {reviewAdded {id}}"}' http://34.148.25.191:8080

curl -i -H 'Content-Type: application/json' -X POST --data-binary @POST.json http://localhost:8080

curl -i -H 'Content-Type: application/json' -X POST\
--data-binary @POST.json http://localhost:8080

curl -i -H 'Content-Type: application/json' -X POST\
--data-binary @testdata/bigquery.graphql.json http://localhost:4000

curl 'http://34.139.171.102' -v\
-H 'accept: multipart/mixed;subscriptionSpec=1.0, application/json'\
-H 'content-type: application/json'\
--data-raw '{"query": "subscription blah {reviewAdded {id product {id sku}}}"}'

curl 'http://35.231.248.199' -v\
-H 'accept: application/json'\
-H 'content-type: application/json'\
--data-binary @queries/0efe7f2513cdfd17a1d6c445976ad9689483026f.graphql.json

curl 'http://34.74.245.219' -v\
-H 'accept: application/json'\
-H 'content-type: application/json'\
--data-binary @queries/0efe7f2513cdfd17a1d6c445976ad9689483026f.graphql.json

curl 'http://localhost:4000' -v\
-H 'accept: application/json'\
-H 'content-type: application/json'\
--data-binary @queries/0efe7f2513cdfd17a1d6c445976ad9689483026f.graphql.json
