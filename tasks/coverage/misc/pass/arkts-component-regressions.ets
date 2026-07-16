@Component
struct ComponentRegression {
  build() {
    Column() {}

    Column() {
      const label = 'value'
      if (label) {
        Text(label)
      }
      this.a('one')
      this.b('two', true)
    }.attribute<number>(1)

    helper()
  }
}

