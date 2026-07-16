@Component
struct InvalidChildren {
  build() {
    Column() { Text('first') Text('second') }
  }
}

